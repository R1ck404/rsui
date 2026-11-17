'use strict';
// Node's `url`: the web's `URL`, what turns paths into `file:` URLs and back, as Windows has paths, and the older
// `url.parse`, `url.format` and `url.resolve` that a lot of packages still use. It's only compiled if something asks for it.

const { URL, URLSearchParams, toASCII, toUnicode } = require('internal/url');

const invalid = (name, message) => Object.assign(new TypeError(message), { code: name });

function fileURLToPath(given, options) {
    const url = typeof given === 'string' ? new URL(given) : given;
    if (!(url instanceof URL)) {
        throw invalid('ERR_INVALID_ARG_TYPE', 'The "path" argument must be of type string or an instance of URL. Received ' + typeof given);
    }
    if (url.protocol !== 'file:') {
        throw invalid('ERR_INVALID_URL_SCHEME', 'The URL must be of scheme file');
    }
    const windows = options?.windows ?? true;
    const pathname = url.pathname;
    if (!windows) {
        if (url.hostname !== '') {
            throw invalid('ERR_INVALID_FILE_URL_HOST', 'File URL host must be "localhost" or empty on linux');
        }
        if (/%2f/i.test(pathname)) {
            throw invalid('ERR_INVALID_FILE_URL_PATH', 'File URL path must not include encoded / characters');
        }
        return decodeURIComponent(pathname);
    }
    if (/%2f|%5c/i.test(pathname)) {
        throw invalid('ERR_INVALID_FILE_URL_PATH', 'File URL path must not include encoded \\ or / characters');
    }
    const path = decodeURIComponent(pathname.replace(/\//g, '\\'));
    // A host is a share on another machine.
    if (url.hostname !== '') {
        return '\\\\' + toUnicode(url.hostname) + path;
    }
    if (!/^\\[a-zA-Z]:/.test(path)) {
        throw invalid('ERR_INVALID_FILE_URL_PATH', 'File URL path must be absolute');
    }
    return path.slice(1);
}

function pathToFileURL(path, options) {
    if (typeof path !== 'string') {
        throw invalid('ERR_INVALID_ARG_TYPE', 'The "path" argument must be of type string. Received ' + typeof path);
    }
    const windows = options?.windows ?? true;
    const nodePath = require('path');
    if (windows && path.startsWith('\\\\')) {
        // `\\server\share\file`, which is the share on that machine, and `\\?\UNC\server\share\file`, which is the same
        // said so its length isn't limited.
        const start = path.startsWith('\\\\?\\UNC\\') ? 8 : 2;
        const hostEnd = path.indexOf('\\', start);
        if (hostEnd === -1 || hostEnd === start) {
            const why = hostEnd === -1 ? 'Missing UNC resource path' : 'Empty UNC servername';
            throw Object.assign(new TypeError(`The argument 'path' ${why}. Received '${path}'`), { code: 'ERR_INVALID_ARG_VALUE' });
        }
        const url = new URL('file://' + path.slice(start, hostEnd));
        url.pathname = path.slice(hostEnd).split('\\').map(encodePart).join('/');
        return url;
    }
    let full = windows ? nodePath.win32.resolve(path) : nodePath.posix.resolve(path);
    // A slash at the end stays, as `resolve` takes it away.
    const last = path[path.length - 1];
    if ((last === '/' || (windows && last === '\\')) && !/[\\/]$/.test(full)) {
        full += windows ? '\\' : '/';
    }
    const segments = windows ? full.split('\\') : full.split('/');
    const url = new URL('file://');
    // Elsewhere a backslash is only a character in a name, which a URL would take as a slash.
    url.pathname = (windows ? '/' : '') + segments.map((part) => (windows ? encodePart(part) : encodePart(part).replace(/\\/g, '%5C'))).join('/');
    return url;
}

// What a path's part has to have written another way to be in a URL's path, as Node writes it. Setting the path writes
// the rest of what Node does that way, as anything that isn't ASCII.
function encodePart(part) {
    return part.replace(/[%\n\r\t#?[\]|~]/g, (found) => '%' + found.charCodeAt(0).toString(16).toUpperCase().padStart(2, '0'));
}

function urlToHttpOptions(url) {
    // Node's has no prototype, and has whatever else the URL has of its own.
    const options = {
        __proto__: null,
        ...url,
        protocol: url.protocol,
        hostname: url.hostname.startsWith('[') ? url.hostname.slice(1, -1) : url.hostname,
        hash: url.hash,
        search: url.search,
        pathname: url.pathname,
        path: `${url.pathname || ''}${url.search || ''}`,
        href: url.href,
    };
    if (url.port !== '') {
        options.port = Number(url.port);
    }
    if (url.username || url.password) {
        options.auth = `${decodeURIComponent(url.username)}:${decodeURIComponent(url.password)}`;
    }
    return options;
}

// Either is '' for a name that can't be one.
function domainToASCII(domain) {
    return toASCII(String(domain));
}

function domainToUnicode(domain) {
    return toUnicode(String(domain));
}

// What's always written another way after the host, in the older API: what RFC 2396 calls unwise, and `'`.
const autoEscape = (text) => text.replace(/[\t\n\r "'<>\\^`{|}]/g, (found) => '%' + found.charCodeAt(0).toString(16).toUpperCase().padStart(2, '0'));

// The older way of taking a URL apart, which is lenient about what it's given, and gives what it found as it was written.
class Url {
    constructor() {
        this.protocol = null;
        this.slashes = null;
        this.auth = null;
        this.host = null;
        this.port = null;
        this.hostname = null;
        this.hash = null;
        this.search = null;
        this.query = null;
        this.pathname = null;
        this.path = null;
        this.href = null;
    }
    parse(text, parseQuery, slashesDenoteHost) {
        if (typeof text !== 'string') {
            throw invalid('ERR_INVALID_ARG_TYPE', 'The "url" argument must be of type string. Received ' + typeof text);
        }
        let rest = text.trim();
        const escape = !/^javascript:/i.test(rest);
        const hashAt = rest.indexOf('#');
        if (hashAt !== -1) {
            this.hash = escape ? autoEscape(rest.slice(hashAt)) : rest.slice(hashAt);
            rest = rest.slice(0, hashAt);
        }
        const queryAt = rest.indexOf('?');
        if (queryAt !== -1) {
            this.search = escape ? autoEscape(rest.slice(queryAt)) : rest.slice(queryAt);
            this.query = this.search.slice(1);
            rest = rest.slice(0, queryAt);
        } else if (parseQuery) {
            this.search = '';
            this.query = {};
        }
        if (parseQuery && typeof this.query === 'string') {
            this.query = require('querystring').parse(this.query);
        }
        const protocol = /^([a-z0-9.+-]+:)/i.exec(rest);
        if (protocol) {
            this.protocol = protocol[1].toLowerCase();
            rest = rest.slice(protocol[1].length);
        }
        const special = ['http:', 'https:', 'ftp:', 'gopher:', 'file:', 'ws:', 'wss:'].includes(this.protocol);
        const slashed = (slashesDenoteHost || protocol || special) && rest.startsWith('//');
        // What isn't written with slashes, like `mailto:`, has its host straight after the scheme.
        if (this.protocol !== 'javascript:' && (slashed || (protocol && !special))) {
            if (slashed) {
                this.slashes = true;
                rest = rest.slice(2);
            }
            let end = rest.search(/[/\\]/);
            if (end === -1) {
                end = rest.length;
            }
            let host = rest.slice(0, end);
            rest = rest.slice(end);
            const authAt = host.lastIndexOf('@');
            if (authAt !== -1) {
                this.auth = decodeURIComponent(host.slice(0, authAt));
                host = host.slice(authAt + 1);
            }
            const port = /:([0-9]*)$/.exec(host);
            if (port) {
                if (port[1] !== '') {
                    this.port = port[1];
                }
                host = host.slice(0, host.length - port[0].length);
            }
            // An IPv6 address keeps its brackets in `host` but not in `hostname`, and a name in another alphabet goes into Punycode.
            const ipv6 = /^\[.*\]$/.test(host);
            this.hostname = ipv6 ? host.slice(1, -1).toLowerCase() : host.toLowerCase();
            if (!ipv6 && /[^\x00-\x7f]/.test(this.hostname)) {
                this.hostname = toASCII(this.hostname) || this.hostname;
            }
            this.host = (ipv6 ? '[' + this.hostname + ']' : this.hostname) + (this.port ? ':' + this.port : '');
            if (special && rest === '') {
                rest = '/';
            }
        }
        if (rest !== '' || this.slashes) {
            this.pathname = rest.replace(/\\/g, '/');
            if (escape) {
                this.pathname = autoEscape(this.pathname);
            }
        }
        if (this.pathname !== null || this.search !== null) {
            this.path = (this.pathname ?? '') + (this.search ?? '');
        }
        this.href = this.format();
        return this;
    }
    format() {
        let auth = this.auth ? encodeURIComponent(this.auth).replace(/%3A/i, ':') + '@' : '';
        let protocol = this.protocol || '';
        let pathname = this.pathname || '';
        let hash = this.hash || '';
        let host = '';
        if (this.host) {
            host = auth + this.host;
        } else if (this.hostname) {
            host = auth + (this.hostname.includes(':') ? '[' + this.hostname + ']' : this.hostname);
            if (this.port) {
                host += ':' + this.port;
            }
        }
        let search = this.search || '';
        if (!search && this.query && typeof this.query === 'object' && Object.keys(this.query).length) {
            search = '?' + require('querystring').stringify(this.query);
        }
        if (protocol && !protocol.endsWith(':')) {
            protocol += ':';
        }
        pathname = pathname.replace(/[?#]/g, encodeURIComponent);
        search = search.replace('#', '%23');
        const special = ['http:', 'https:', 'ftp:', 'gopher:', 'file:', 'ws:', 'wss:'].includes(protocol);
        if (this.slashes || ((!protocol || special) && host !== '')) {
            host = '//' + host;
            if (pathname && pathname[0] !== '/') {
                pathname = '/' + pathname;
            }
        }
        if (hash && hash[0] !== '#') {
            hash = '#' + hash;
        }
        if (search && search[0] !== '?') {
            search = '?' + search;
        }
        return protocol + host + pathname + search + hash;
    }
    resolve(relative) {
        return new URL(relative, new URL(this.href, 'resolve://')).href.replace(/^resolve:\/\/\/?/, this.href.startsWith('/') ? '/' : '');
    }
}

function parse(text, parseQuery, slashesDenoteHost) {
    if (text instanceof Url) {
        return text;
    }
    return new Url().parse(text, parseQuery, slashesDenoteHost);
}

function format(object, options) {
    if (typeof object === 'string') {
        return parse(object).format();
    }
    if (object instanceof URL) {
        const url = new URL(object.href);
        if (options?.auth === false) {
            url.username = '';
            url.password = '';
        }
        if (options?.search === false) {
            url.search = '';
        }
        if (options?.fragment === false) {
            url.hash = '';
        }
        if (options?.unicode && url.hostname !== '') {
            // The host as people write it, which `URL` itself would put back into Punycode.
            const at = url.href.indexOf(url.host, url.protocol.length);
            return url.href.slice(0, at) + toUnicode(url.hostname) + (url.port ? ':' + url.port : '') + url.href.slice(at + url.host.length);
        }
        return url.href;
    }
    return Url.prototype.format.call(object);
}

function resolve(from, to) {
    // Resolving against something that isn't a whole URL keeps it that way.
    const base = /^[a-z][a-z0-9.+-]*:/i.test(from) ? from : 'resolve:' + (from.startsWith('/') ? '' : '/') + from;
    // What has no path to go from, like `mailto:a@b`, keeps its scheme and takes the rest from what's resolved.
    const scheme = /^[a-z][a-z0-9.+-]*:/i.exec(base)[0];
    if (!/^[a-z][a-z0-9.+-]*:/i.test(to) && !URL.canParse(to, base)) {
        return scheme.toLowerCase() + to;
    }
    const resolved = new URL(to, base).href;
    if (base === from) {
        return resolved;
    }
    const path = resolved.slice('resolve:'.length);
    return from.startsWith('/') ? path : path.replace(/^\//, '');
}

module.exports = {
    URL,
    URLSearchParams,
    Url,
    parse,
    format,
    resolve,
    resolveObject: (from, to) => parse(resolve(from, to)),
    fileURLToPath,
    pathToFileURL,
    urlToHttpOptions,
    domainToASCII,
    domainToUnicode,
};
