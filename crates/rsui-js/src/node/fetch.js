'use strict';
// `fetch`, `Request`, `Response` and `Headers`. It's only compiled the first time something asks for one of them, and each
// request happens on a thread of its own. What comes back arrives as it arrives: the promise from `fetch` is kept once the
// headers are in, and then each part of the body is read only when something asks for it. A body only becomes a
// `ReadableStream` if something looks at `body`, as reading it whole doesn't need one.

const webStreams = () => require('internal/misc/web_streams');

function normalizeName(name) {
    const text = String(name);
    if (!/^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(text)) {
        throw new TypeError(`Invalid header name: "${text}"`);
    }
    return text.toLowerCase();
}

function normalizeValue(value) {
    return String(value).replace(/^[\t\n\r ]+|[\t\n\r ]+$/g, '');
}

class Headers {
    #items = new Map();
    #cookies = [];
    constructor(from = undefined) {
        if (from === undefined || from === null) {
            return;
        }
        if (typeof from !== 'object' && typeof from !== 'function') {
            throw new TypeError('Headers must be made from an object or a list of pairs');
        }
        if (from instanceof Headers) {
            for (const [name, value] of from.#items) {
                this.#items.set(name, value);
            }
            this.#cookies = [...from.#cookies];
        } else if (typeof from[Symbol.iterator] === 'function') {
            for (const pair of from) {
                const [name, value, ...rest] = pair;
                if (rest.length > 0 || pair.length !== 2) {
                    throw new TypeError('Each header must be a pair of a name and a value');
                }
                this.append(name, value);
            }
        } else {
            for (const name of Object.keys(from)) {
                this.append(name, from[name]);
            }
        }
    }
    get(name) {
        const key = normalizeName(name);
        if (key === 'set-cookie') {
            return this.#cookies.length > 0 ? this.#cookies.join(', ') : null;
        }
        const found = this.#items.get(key);
        return found === undefined ? null : found;
    }
    getSetCookie() {
        return [...this.#cookies];
    }
    has(name) {
        const key = normalizeName(name);
        return key === 'set-cookie' ? this.#cookies.length > 0 : this.#items.has(key);
    }
    set(name, value) {
        const key = normalizeName(name);
        const text = normalizeValue(value);
        if (key === 'set-cookie') {
            this.#cookies = [text];
        } else {
            this.#items.set(key, text);
        }
    }
    append(name, value) {
        const key = normalizeName(name);
        const text = normalizeValue(value);
        if (key === 'set-cookie') {
            this.#cookies.push(text);
            return;
        }
        const old = this.#items.get(key);
        this.#items.set(key, old === undefined ? text : old + ', ' + text);
    }
    delete(name) {
        const key = normalizeName(name);
        if (key === 'set-cookie') {
            this.#cookies = [];
        } else {
            this.#items.delete(key);
        }
    }
    // As the standard says, they're in order of name, and each cookie is its own.
    #sorted() {
        const out = [...this.#items];
        for (const cookie of this.#cookies) {
            out.push(['set-cookie', cookie]);
        }
        return out.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
    }
    forEach(run, self = undefined) {
        for (const [name, value] of this.#sorted()) {
            run.call(self, value, name, this);
        }
    }
    entries() {
        return this.#sorted()[Symbol.iterator]();
    }
    keys() {
        return this.#sorted().map(([name]) => name)[Symbol.iterator]();
    }
    values() {
        return this.#sorted().map(([, value]) => value)[Symbol.iterator]();
    }
    [Symbol.iterator]() {
        return this.entries();
    }
    get [Symbol.toStringTag]() {
        return 'Headers';
    }
}

// Where a body's bytes come from, when it isn't a stream: `next()` gives each part, and `null` at the end.
function bytesSource(bytes) {
    let left = bytes;
    return {
        bytes,
        next() {
            const part = left;
            left = null;
            return Promise.resolve(part !== null && part.length > 0 ? part : null);
        },
        cancel() {
            left = null;
        },
    };
}

// A request that's being read stops if nothing can reach what reads it any more.
let forgotten = null;

function networkSource(id) {
    const source = {
        bytes: null,
        reason: undefined,
        done: false,
        controller: null,
        next() {
            if (source.reason !== undefined) {
                return Promise.reject(source.reason);
            }
            return __fs.httpRead(id).then(
                (part) => {
                    if (part === null) {
                        source.done = true;
                    }
                    return part;
                },
                (error) => {
                    source.done = true;
                    throw source.reason !== undefined ? source.reason : error;
                },
            );
        },
        cancel() {
            if (!source.done) {
                source.done = true;
                __fs.httpCancel(id);
            }
        },
        abort(reason) {
            if (source.done) {
                return;
            }
            source.reason = reason;
            source.cancel();
            if (source.controller !== null) {
                source.controller.error(reason);
            }
        },
    };
    forgotten ??= new FinalizationRegistry((number) => __fs.httpCancel(number));
    forgotten.register(source, id);
    return source;
}

function streamFrom(source) {
    const { ReadableStream } = webStreams();
    return new ReadableStream({
        type: 'bytes',
        start(controller) {
            source.controller = controller;
        },
        pull(controller) {
            return source.next().then((part) => {
                if (part === null) {
                    controller.close();
                    controller.byobRequest?.respond(0);
                } else {
                    controller.enqueue(part);
                }
            });
        },
        cancel(reason) {
            source.cancel(reason);
        },
    }, { highWaterMark: 0 });
}

// What a request or a response has to say for itself, made into a stream only if something asks for one.
class Body {
    constructor(source, stream) {
        this.source = source;
        this.stream = stream;
        this.used = false;
    }
    get readable() {
        if (this.stream === null) {
            this.stream = streamFrom(this.source);
            this.source = null;
        }
        return this.stream;
    }
    get disturbed() {
        return this.stream !== null ? webStreams().internals.isDisturbed(this.stream) : this.used;
    }
    get locked() {
        return this.stream !== null && this.stream.locked;
    }
}

// A form's fields, each a string or a file. A blob that's given becomes a file called `blob`, as the standard says.
class FormData {
    #entries = [];
    constructor(form = undefined) {
        if (form !== undefined) {
            throw new TypeError('FormData can only be made empty here, as there are no HTML forms');
        }
    }
    static #entry(name, value, filename) {
        if (typeof Blob === 'function' && value instanceof Blob) {
            if (!(value instanceof File) || filename !== undefined) {
                value = new File([value], filename === undefined ? (value instanceof File ? value.name : 'blob') : String(filename), { type: value.type, lastModified: value.lastModified });
            }
            return [String(name), value];
        }
        return [String(name), String(value)];
    }
    append(name, value, filename = undefined) {
        this.#entries.push(FormData.#entry(name, value, filename));
    }
    set(name, value, filename = undefined) {
        const entry = FormData.#entry(name, value, filename);
        const at = this.#entries.findIndex(([key]) => key === entry[0]);
        if (at < 0) {
            this.#entries.push(entry);
            return;
        }
        this.#entries[at] = entry;
        this.#entries = this.#entries.filter(([key], index) => index <= at || key !== entry[0]);
    }
    get(name) {
        const found = this.#entries.find(([key]) => key === String(name));
        return found === undefined ? null : found[1];
    }
    getAll(name) {
        return this.#entries.filter(([key]) => key === String(name)).map(([, value]) => value);
    }
    has(name) {
        return this.#entries.some(([key]) => key === String(name));
    }
    delete(name) {
        this.#entries = this.#entries.filter(([key]) => key !== String(name));
    }
    forEach(run, self = undefined) {
        for (const [name, value] of this.entries()) {
            run.call(self, value, name, this);
        }
    }
    entries() {
        return this.#entries.map(([name, value]) => [name, value])[Symbol.iterator]();
    }
    keys() {
        return this.#entries.map(([name]) => name)[Symbol.iterator]();
    }
    values() {
        return this.#entries.map(([, value]) => value)[Symbol.iterator]();
    }
    [Symbol.iterator]() {
        return this.entries();
    }
    get [Symbol.toStringTag]() {
        return 'FormData';
    }
}

// A form as `multipart/form-data`, with each name and file name escaped as the standard says.
function multipart(form, boundary) {
    const encoder = new TextEncoder();
    const escape = (text) => text.replace(/\n/g, '%0A').replace(/\r/g, '%0D').replace(/"/g, '%22');
    const lines = (text) => text.replace(/\r\n|\r|\n/g, '\r\n');
    const parts = [];
    for (const [name, value] of form) {
        let head = `--${boundary}\r\nContent-Disposition: form-data; name="${escape(lines(name))}"`;
        if (typeof value === 'string') {
            parts.push(encoder.encode(head + '\r\n\r\n' + lines(value) + '\r\n'));
        } else {
            head += `; filename="${escape(value.name)}"\r\nContent-Type: ${value.type || 'application/octet-stream'}\r\n\r\n`;
            parts.push(encoder.encode(head), value.bytes, encoder.encode('\r\n'));
        }
    }
    parts.push(encoder.encode(`--${boundary}--\r\n`));
    return concatenate(parts);
}

// A body that's a form, either kind, back into its fields.
function parseForm(bytes, type) {
    const decoder = new TextDecoder();
    const form = new FormData();
    if (/^application\/x-www-form-urlencoded/i.test(type)) {
        for (const [name, value] of new URLSearchParams(decoder.decode(bytes))) {
            form.append(name, value);
        }
        return form;
    }
    const found = /^multipart\/form-data;.*boundary=(?:"([^"]+)"|([^;]+))/i.exec(type);
    if (found === null) {
        throw new TypeError('The body is not a form');
    }
    // Each byte as a character, to look for where parts start and end, which are the same places in the bytes.
    let text = '';
    for (let at = 0; at < bytes.length; at += 8192) {
        text += String.fromCharCode(...bytes.subarray(at, at + 8192));
    }
    const fromBytes = (latin) => decoder.decode(Uint8Array.from(latin, (character) => character.charCodeAt(0)));
    const delimiter = '--' + (found[1] ?? found[2].trim());
    let at = text.indexOf(delimiter);
    while (at >= 0 && !text.startsWith('--', at + delimiter.length)) {
        const start = at + delimiter.length + 2;
        const headEnd = text.indexOf('\r\n\r\n', start);
        const end = text.indexOf('\r\n' + delimiter, headEnd + 4);
        if (headEnd < 0 || end < 0) {
            throw new TypeError('The form is cut off');
        }
        const head = text.slice(start, headEnd);
        const disposition = /content-disposition:[^\r\n]*?\bname="([^"]*)"(?:;\s*filename="([^"]*)")?/i.exec(head);
        if (disposition === null) {
            throw new TypeError('A part of the form has no name');
        }
        const content = bytes.subarray(headEnd + 4, end);
        const name = fromBytes(disposition[1]);
        if (disposition[2] !== undefined) {
            const contentType = /content-type:\s*([^\r\n]*)/i.exec(head);
            form.append(name, new File([content], fromBytes(disposition[2]), { type: contentType === null ? '' : contentType[1] }));
        } else {
            form.append(name, decoder.decode(content));
        }
        at = end + 2;
    }
    return form;
}

function copyOf(view) {
    return new Uint8Array(view.buffer.slice(view.byteOffset, view.byteOffset + view.byteLength));
}

// What's given as a body, as the standard says to take it: a body, and the type it says it is, if it says.
function extractBody(given, keepalive) {
    if (typeof given === 'string') {
        return [new Body(bytesSource(new TextEncoder().encode(given)), null), 'text/plain;charset=UTF-8'];
    }
    if (given instanceof ArrayBuffer) {
        return [new Body(bytesSource(new Uint8Array(given.slice(0))), null), null];
    }
    if (ArrayBuffer.isView(given)) {
        return [new Body(bytesSource(copyOf(given)), null), null];
    }
    if (typeof given === 'object' && given !== null) {
        if (typeof URLSearchParams === 'function' && given instanceof URLSearchParams) {
            return [new Body(bytesSource(new TextEncoder().encode(String(given))), null), 'application/x-www-form-urlencoded;charset=UTF-8'];
        }
        if (given instanceof FormData) {
            const boundary = '----formdata-rsui-' + Math.random().toString(16).slice(2) + Math.random().toString(16).slice(2);
            return [new Body(bytesSource(multipart(given, boundary)), null), 'multipart/form-data; boundary=' + boundary];
        }
        if (typeof Blob === 'function' && given instanceof Blob) {
            return [new Body(bytesSource(given.bytes.slice()), null), given.type === '' ? null : given.type];
        }
        // The streams are only compiled for what could be one.
        if (typeof given.getReader === 'function' && webStreams().internals.isReadable(given)) {
            if (keepalive) {
                throw new TypeError('A keepalive request can\'t have a stream for its body');
            }
            if (given.locked || webStreams().internals.isDisturbed(given)) {
                throw new TypeError('The body\'s stream has already been read or is locked');
            }
            return [new Body(null, given), null];
        }
    }
    return [new Body(bytesSource(new TextEncoder().encode(String(given))), null), 'text/plain;charset=UTF-8'];
}

function unusable() {
    return new TypeError('Body is unusable: Body has already been read');
}

function concatenate(parts) {
    if (parts.length === 1) {
        return parts[0];
    }
    const out = new Uint8Array(parts.reduce((total, part) => total + part.byteLength, 0));
    let at = 0;
    for (const part of parts) {
        out.set(part, at);
        at += part.byteLength;
    }
    return out;
}

// Reads all of a body, for a promise of its bytes, which nothing else has.
function consume(body) {
    if (body === null) {
        return Promise.resolve(new Uint8Array(0));
    }
    if (body.disturbed || body.locked) {
        return Promise.reject(unusable());
    }
    const parts = [];
    if (body.stream === null) {
        body.used = true;
        const source = body.source;
        if (source.bytes !== null) {
            source.cancel();
            return Promise.resolve(source.bytes);
        }
        const more = () => source.next().then((part) => {
            if (part === null) {
                return concatenate(parts);
            }
            parts.push(part);
            return more();
        });
        return more();
    }
    const reader = body.stream.getReader();
    const more = () => reader.read().then(({ value, done }) => {
        if (done) {
            return concatenate(parts);
        }
        if (!(value instanceof Uint8Array)) {
            throw new TypeError('A body\'s stream must give Uint8Arrays');
        }
        parts.push(value);
        return more();
    });
    return more();
}

// Lets two read the same body: this one keeps one half, and the other is given the other half.
function teeBody(body) {
    if (body.disturbed || body.locked) {
        throw new TypeError('A body that has been read can\'t be cloned');
    }
    if (body.stream === null && body.source.bytes !== null) {
        return new Body(bytesSource(body.source.bytes), null);
    }
    const [mine, theirs] = body.readable.tee();
    body.stream = mine;
    return new Body(null, theirs);
}

// What reads a body, the same for requests and responses, which each say where their body is.
function addBodyReading(made, recordOf) {
    const reading = {
        get body() {
            const record = recordOf(this);
            return record === null ? null : record.readable;
        },
        get bodyUsed() {
            const record = recordOf(this);
            return record !== null && record.disturbed;
        },
        arrayBuffer() {
            return consume(recordOf(this)).then((bytes) =>
                bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength ? bytes.buffer : bytes.slice().buffer);
        },
        blob() {
            const type = this.headers.get('content-type') ?? '';
            return consume(recordOf(this)).then((bytes) => new Blob([bytes], { type }));
        },
        bytes() {
            return consume(recordOf(this));
        },
        formData() {
            const type = this.headers.get('content-type') ?? '';
            return consume(recordOf(this)).then((bytes) => parseForm(bytes, type));
        },
        json() {
            return consume(recordOf(this)).then((bytes) => JSON.parse(new TextDecoder().decode(bytes)));
        },
        text() {
            return consume(recordOf(this)).then((bytes) => new TextDecoder().decode(bytes));
        },
    };
    Object.defineProperties(made.prototype, Object.getOwnPropertyDescriptors(reading));
}

const NO_BODY_STATUSES = [101, 103, 204, 205, 304];
const REDIRECT_STATUSES = [301, 302, 303, 307, 308];
const STANDARD_METHODS = ['DELETE', 'GET', 'HEAD', 'OPTIONS', 'POST', 'PUT', 'PATCH'];

let adoptResponse;
let responseBody;

class Response {
    #body = null;
    #status = 200;
    #statusText = '';
    #headers;
    #url = '';
    #redirected = false;
    #type = 'default';

    constructor(body = null, init = undefined) {
        const options = init ?? {};
        const status = options.status === undefined ? 200 : Number(options.status);
        if (!Number.isInteger(status) || status < 200 || status > 599) {
            throw new RangeError(`init["status"] must be in the range of 200 to 599, inclusive.`);
        }
        this.#status = status;
        this.#statusText = options.statusText === undefined ? '' : String(options.statusText);
        this.#headers = new Headers(options.headers);
        if (body !== null && body !== undefined) {
            if (NO_BODY_STATUSES.includes(status)) {
                throw new TypeError('A response with this status can\'t have a body');
            }
            const [made, type] = extractBody(body, false);
            this.#body = made;
            if (type !== null && !this.#headers.has('content-type')) {
                this.#headers.set('content-type', type);
            }
        }
    }

    static {
        adoptResponse = (response, body, url, redirected) => {
            response.#body = body;
            response.#url = url;
            response.#redirected = redirected;
            response.#type = 'basic';
        };
        responseBody = (response) => response.#body;
    }

    static error() {
        const made = new Response();
        made.#status = 0;
        made.#type = 'error';
        return made;
    }

    static json(data, init = undefined) {
        const text = JSON.stringify(data);
        if (text === undefined) {
            throw new TypeError('The value can\'t be written as JSON');
        }
        const made = new Response(text, init);
        if (!(init?.headers !== undefined && new Headers(init.headers).has('content-type'))) {
            made.#headers.set('content-type', 'application/json');
        }
        return made;
    }

    static redirect(url, status = 302) {
        const parsed = new URL(String(url));
        if (!REDIRECT_STATUSES.includes(status)) {
            throw new RangeError(`Invalid status code ${status}`);
        }
        const made = new Response(null, { status });
        made.#headers.set('location', parsed.href);
        return made;
    }

    get type() {
        return this.#type;
    }
    get url() {
        return this.#url;
    }
    get redirected() {
        return this.#redirected;
    }
    get status() {
        return this.#status;
    }
    get ok() {
        return this.#status >= 200 && this.#status <= 299;
    }
    get statusText() {
        return this.#statusText;
    }
    get headers() {
        return this.#headers;
    }

    clone() {
        const made = new Response(null, { status: this.#status === 0 ? 200 : this.#status, statusText: this.#statusText, headers: this.#headers });
        made.#status = this.#status;
        made.#type = this.#type;
        made.#url = this.#url;
        made.#redirected = this.#redirected;
        made.#body = this.#body === null ? null : teeBody(this.#body);
        return made;
    }

    get [Symbol.toStringTag]() {
        return 'Response';
    }
}
addBodyReading(Response, (response) => responseBody(response));

let requestBody;
let requestSignal;

class Request {
    #url;
    #method = 'GET';
    #headers;
    #body = null;
    #signal = null;
    #keepalive = false;
    #redirect = 'follow';
    #duplex = 'half';

    constructor(input, init = undefined) {
        const options = init ?? {};
        if (typeof options !== 'object' && typeof options !== 'function') {
            throw new TypeError('A request\'s options must be an object');
        }
        let inputBody = null;
        if (input instanceof Request) {
            this.#url = input.#url;
            this.#method = input.#method;
            this.#headers = new Headers(input.#headers);
            this.#signal = input.#signal;
            this.#keepalive = input.#keepalive;
            this.#redirect = input.#redirect;
            inputBody = input.#body;
        } else {
            const parsed = new URL(String(input));
            if (parsed.username !== '' || parsed.password !== '') {
                throw new TypeError('Request can\'t be made from a URL with a name or password in it');
            }
            this.#url = parsed.href;
            this.#headers = new Headers();
        }
        if (options.method !== undefined) {
            const method = String(options.method);
            if (!/^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(method)) {
                throw new TypeError(`'${method}' is not a valid HTTP method.`);
            }
            const upper = method.toUpperCase();
            if (upper === 'CONNECT' || upper === 'TRACE' || upper === 'TRACK') {
                throw new TypeError(`'${method}' HTTP method is unsupported.`);
            }
            this.#method = STANDARD_METHODS.includes(upper) && upper !== 'PATCH' ? upper : method;
        }
        if (options.headers !== undefined) {
            this.#headers = new Headers(options.headers);
        }
        if (options.signal !== undefined && options.signal !== null) {
            this.#signal = options.signal;
        }
        if (options.keepalive !== undefined) {
            this.#keepalive = Boolean(options.keepalive);
        }
        if (options.redirect !== undefined) {
            this.#redirect = String(options.redirect);
        }
        const given = options.body;
        if ((given !== undefined && given !== null) || inputBody !== null) {
            if (this.#method === 'GET' || this.#method === 'HEAD') {
                throw new TypeError('Request with GET/HEAD method cannot have body.');
            }
        }
        if (given !== undefined && given !== null) {
            const [made, type] = extractBody(given, this.#keepalive);
            if (made.stream !== null && options.duplex !== 'half') {
                throw new TypeError('RequestInit: duplex option is required when sending a body.');
            }
            this.#body = made;
            if (type !== null && !this.#headers.has('content-type')) {
                this.#headers.set('content-type', type);
            }
        } else if (inputBody !== null) {
            if (inputBody.disturbed || inputBody.locked) {
                throw new TypeError('Cannot construct a Request with a Request object that has already been used.');
            }
            // The body moves to this request, and the one it came from can't be read any more.
            this.#body = inputBody;
            input.#body = new Body(bytesSource(new Uint8Array(0)), null);
            input.#body.used = true;
        }
    }

    static {
        requestBody = (request) => request.#body;
        requestSignal = (request) => request.#signal;
    }

    get method() {
        return this.#method;
    }
    get url() {
        return this.#url;
    }
    get headers() {
        return this.#headers;
    }
    get signal() {
        this.#signal ??= new AbortController().signal;
        return this.#signal;
    }
    get keepalive() {
        return this.#keepalive;
    }
    get redirect() {
        return this.#redirect;
    }
    get duplex() {
        return this.#duplex;
    }
    get cache() {
        return 'default';
    }
    get credentials() {
        return 'same-origin';
    }
    get destination() {
        return '';
    }
    get integrity() {
        return '';
    }
    get mode() {
        return 'cors';
    }
    get referrer() {
        return 'about:client';
    }
    get referrerPolicy() {
        return '';
    }

    clone() {
        if (this.#body !== null && (this.#body.disturbed || this.#body.locked)) {
            throw new TypeError('A request whose body has been read can\'t be cloned');
        }
        const made = new Request(this.#url, { method: this.#method, headers: this.#headers, keepalive: this.#keepalive, redirect: this.#redirect });
        made.#signal = this.#signal;
        made.#body = this.#body === null ? null : teeBody(this.#body);
        return made;
    }

    get [Symbol.toStringTag]() {
        return 'Request';
    }
}
addBodyReading(Request, (request) => requestBody(request));

function fetch(input, init = undefined) {
    let request;
    try {
        request = new Request(input, init);
    } catch (error) {
        return Promise.reject(error);
    }
    const signal = requestSignal(request);
    if (signal !== null && signal.aborted) {
        return Promise.reject(signal.reason);
    }
    const url = request.url;
    const headers = [];
    request.headers.forEach((value, name) => headers.push([name, value]));
    const body = requestBody(request);
    // A body that's a stream is read whole before the request is sent.
    const sending = body === null ? Promise.resolve(undefined) : consume(body);
    return sending.then((bytes) => {
        if (signal !== null && signal.aborted) {
            throw signal.reason;
        }
        const [id, head] = __fs.httpStart(url, request.method, headers, bytes);
        const source = networkSource(id);
        return new Promise((resolve, reject) => {
            if (signal !== null) {
                signal.addEventListener('abort', () => {
                    source.abort(signal.reason);
                    reject(signal.reason);
                }, { once: true });
            }
            head.then((answer) => {
                if (source.reason !== undefined) {
                    return;
                }
                const response = new Response(null, { status: answer.status < 200 || answer.status > 599 ? 200 : answer.status, statusText: answer.statusText, headers: answer.headers });
                const final = answer.url === '' ? url : new URL(answer.url).href;
                const empty = request.method === 'HEAD' || NO_BODY_STATUSES.includes(answer.status);
                if (empty) {
                    source.cancel();
                }
                adoptResponse(response, empty ? null : new Body(source, null), final, final !== url);
                resolve(response);
            }, (error) => reject(source.reason !== undefined ? source.reason : error));
        });
    });
}

module.exports = { fetch, FormData, Headers, Request, Response };
