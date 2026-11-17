'use strict';
// Node's `http` and `https`: asking for things, on what `fetch` is written with, and a simple server. A request takes what's
// being sent all in one go, and gives back what comes as it comes, as Windows does the rest.

const EventEmitter = require('events');
const { Readable, Writable } = require('stream');

const STATUS_CODES = {
    100: 'Continue', 101: 'Switching Protocols', 200: 'OK', 201: 'Created', 202: 'Accepted', 204: 'No Content', 206: 'Partial Content',
    301: 'Moved Permanently', 302: 'Found', 303: 'See Other', 304: 'Not Modified', 307: 'Temporary Redirect', 308: 'Permanent Redirect',
    400: 'Bad Request', 401: 'Unauthorized', 403: 'Forbidden', 404: 'Not Found', 405: 'Method Not Allowed', 408: 'Request Timeout',
    409: 'Conflict', 410: 'Gone', 413: 'Payload Too Large', 415: 'Unsupported Media Type', 418: "I'm a Teapot", 422: 'Unprocessable Entity',
    429: 'Too Many Requests', 500: 'Internal Server Error', 501: 'Not Implemented', 502: 'Bad Gateway', 503: 'Service Unavailable', 504: 'Gateway Timeout',
};
const METHODS = ['DELETE', 'GET', 'HEAD', 'OPTIONS', 'PATCH', 'POST', 'PUT'];

class Agent extends EventEmitter {
    constructor(options) {
        super();
        this.options = { ...options };
        this.defaultPort = 80;
        this.protocol = 'http:';
    }
    destroy() {}
}

function joined(parts) {
    const { Buffer } = require('internal/buffer');
    return Buffer.concat(parts);
}

// What came back, which reads the next part of its body only when it has room for it, so that pausing it stops the
// request reading any more until it's resumed.
function messageFrom(answer, url, id) {
    const { Buffer } = require('internal/buffer');
    let ended = false;
    const message = new Readable({
        read() {
            __fs.httpRead(id).then(
                (part) => {
                    if (message.destroyed) {
                        return;
                    }
                    if (part === null) {
                        ended = true;
                        message.complete = true;
                        message.push(null);
                    } else {
                        message.push(Buffer.from(part.buffer, part.byteOffset, part.byteLength));
                    }
                },
                (error) => {
                    if (!message.destroyed) {
                        error.code ??= 'ECONNRESET';
                        message.destroy(error);
                    }
                },
            );
        },
        destroy(error, done) {
            if (!ended) {
                ended = true;
                __fs.httpCancel(id);
            }
            done(error);
        },
    });
    message.statusCode = answer.status;
    message.statusMessage = answer.statusText || STATUS_CODES[answer.status] || '';
    message.httpVersion = '1.1';
    message.httpVersionMajor = 1;
    message.httpVersionMinor = 1;
    message.url = url;
    message.headers = {};
    message.rawHeaders = [];
    for (const [name, value] of answer.headers) {
        const lower = name.toLowerCase();
        message.rawHeaders.push(name, value);
        if (lower === 'set-cookie') {
            (message.headers[lower] ??= []).push(value);
        } else {
            message.headers[lower] = lower in message.headers ? `${message.headers[lower]}, ${value}` : value;
        }
    }
    message.complete = false;
    message.aborted = false;
    message.socket = message.connection = null;
    message.setTimeout = () => message;
    return message;
}

function wholeUrl(options, protocol) {
    if (options.href) {
        return options.href;
    }
    const scheme = options.protocol ?? protocol;
    const host = options.hostname ?? options.host ?? 'localhost';
    const bracketed = host.includes(':') && !host.startsWith('[') ? `[${host}]` : host;
    const port = options.port ? `:${options.port}` : '';
    return `${scheme}//${bracketed}${port}${options.path ?? '/'}`;
}

class ClientRequest extends Writable {
    constructor(options, protocol, listener) {
        const held = [];
        super({
            write(chunk, encoding, done) {
                held.push(chunk);
                done();
            },
            final(done) {
                done();
                this._send(joined(held));
            },
        });
        this.method = String(options.method ?? 'GET').toUpperCase();
        this.path = options.path ?? '/';
        this.host = options.hostname ?? options.host ?? 'localhost';
        this.protocol = options.protocol ?? protocol;
        this.aborted = false;
        this.reusedSocket = false;
        this.timeoutAfter = options.timeout;
        this.autoDestroy = false;
        this._url = wholeUrl(options, protocol);
        this._headers = new Map();
        for (const [name, value] of Object.entries(options.headers ?? {})) {
            this.setHeader(name, value);
        }
        if (!this.hasHeader('host')) {
            const bare = new URL(this._url);
            this.setHeader('Host', bare.host);
        }
        if (options.auth && !this.hasHeader('authorization')) {
            this.setHeader('Authorization', 'Basic ' + btoa(options.auth));
        }
        if (listener) {
            this.once('response', listener);
        }
        if (options.signal) {
            options.signal.addEventListener('abort', () => this.destroy(options.signal.reason), { once: true });
        }
    }

    setHeader(name, value) {
        this._headers.set(String(name).toLowerCase(), [String(name), Array.isArray(value) ? value.join(', ') : String(value)]);
        return this;
    }
    getHeader(name) {
        return this._headers.get(String(name).toLowerCase())?.[1];
    }
    getHeaders() {
        return Object.fromEntries([...this._headers].map(([name, [, value]]) => [name, value]));
    }
    getHeaderNames() {
        return [...this._headers.keys()];
    }
    hasHeader(name) {
        return this._headers.has(String(name).toLowerCase());
    }
    removeHeader(name) {
        this._headers.delete(String(name).toLowerCase());
    }
    flushHeaders() {}
    setNoDelay() {}
    setSocketKeepAlive() {}
    setTimeout(milliseconds, listener) {
        this.timeoutAfter = milliseconds;
        if (listener) {
            this.once('timeout', listener);
        }
        return this;
    }
    abort() {
        this.aborted = true;
        this.destroy();
    }

    // Stopping a request stops it wherever it's got to, and what's come back so far is cut off, as in Node.
    _destroy(error, done) {
        if (this._id !== undefined) {
            __fs.httpCancel(this._id);
        }
        const message = this.res;
        const cutOff = message && !message.complete && !message.destroyed;
        if (cutOff) {
            message.aborted = true;
            message.emit('aborted');
        }
        done(error);
        if (cutOff) {
            message.destroy(Object.assign(new Error('aborted'), { code: 'ECONNRESET' }));
        }
    }

    _send(body) {
        if (this.destroyed) {
            return;
        }
        let timer = null;
        if (this.timeoutAfter > 0) {
            timer = setTimeout(() => this.emit('timeout'), this.timeoutAfter);
        }
        const headers = [...this._headers.values()];
        const [id, head] = __fs.httpStart(this._url, this.method, headers, body);
        this._id = id;
        head.then(
            (answer) => {
                clearTimeout(timer);
                if (this.destroyed) {
                    return;
                }
                const message = messageFrom(answer, this._url, id);
                this.res = message;
                message.once('close', () => {
                    if (!this.destroyed) {
                        this.emit('close');
                    }
                });
                if (!this.emit('response', message)) {
                    message.resume();
                }
            },
            (error) => {
                clearTimeout(timer);
                if (!this.destroyed) {
                    error.code ??= 'ECONNREFUSED';
                    this.emit('error', error);
                }
            },
        );
    }
}

/// A server, for what a program offers on its own machine. Each connection is for one request.
class Server extends require('events') {
    #sockets = new Set();
    #server;

    constructor(onRequest) {
        super();
        if (typeof onRequest === 'function') {
            this.on('request', onRequest);
        }
        const { net } = require('internal/net');
        this.#server = net.createServer((socket) => this.#connected(socket));
        this.#server.on('listening', () => this.emit('listening'));
        this.#server.on('error', (error) => this.emit('error', error));
        this.#server.on('close', () => this.emit('close'));
    }

    #connected(socket) {
        this.#sockets.add(socket);
        socket.on('close', () => this.#sockets.delete(socket));
        socket.on('error', () => {});
        const { Buffer } = require('internal/buffer');
        let held = Buffer.alloc(0);
        let request = null;
        let wanted = 0;
        let seen = 0;
        socket.on('data', (chunk) => {
            if (request !== null) {
                seen += chunk.length;
                request.push(chunk);
                if (seen >= wanted) {
                    request.push(null);
                }
                return;
            }
            held = Buffer.concat([held, chunk]);
            const end = held.indexOf('\r\n\r\n');
            if (end < 0) {
                return;
            }
            const [first, ...lines] = held.subarray(0, end).toString('latin1').split('\r\n');
            const [method, url, version] = first.split(' ');
            request = new (require('stream').Readable)({ read() {} });
            request.method = method;
            request.url = url;
            request.httpVersion = (version ?? '').replace('HTTP/', '');
            request.headers = {};
            request.rawHeaders = [];
            for (const line of lines) {
                const colon = line.indexOf(':');
                if (colon > 0) {
                    const name = line.slice(0, colon).trim();
                    const value = line.slice(colon + 1).trim();
                    request.headers[name.toLowerCase()] = value;
                    request.rawHeaders.push(name, value);
                }
            }
            request.socket = request.connection = socket;
            wanted = Number(request.headers['content-length'] ?? 0);
            const rest = held.subarray(end + 4);
            seen = rest.length;
            if (rest.length > 0) {
                request.push(rest);
            }
            if (seen >= wanted) {
                request.push(null);
            }
            this.emit('request', request, new ServerResponse(socket));
        });
    }

    listen(...given) {
        this.#server.listen(...given);
        return this;
    }

    close(done) {
        for (const socket of this.#sockets) {
            socket.destroy();
        }
        this.#server.close(done);
        return this;
    }

    async [Symbol.asyncDispose]() {
        await new Promise((resolve, reject) => this.close((error) => (error ? reject(error) : resolve())));
    }

    address() {
        return this.#server.address();
    }

    get listening() {
        return this.#server.listening;
    }

    ref() {
        return this;
    }

    unref() {
        return this;
    }

    setTimeout() {
        return this;
    }
}

class ServerResponse extends require('stream').Writable {
    #socket;
    #headers = new Map();
    #started = false;
    #chunked = false;

    constructor(socket) {
        super();
        this.#socket = socket;
        this.socket = socket;
        this.statusCode = 200;
        this.statusMessage = undefined;
        this.headersSent = false;
    }

    setHeader(name, value) {
        this.#headers.set(String(name).toLowerCase(), [name, value]);
        return this;
    }

    getHeader(name) {
        return this.#headers.get(String(name).toLowerCase())?.[1];
    }

    hasHeader(name) {
        return this.#headers.has(String(name).toLowerCase());
    }

    removeHeader(name) {
        this.#headers.delete(String(name).toLowerCase());
    }

    getHeaders() {
        return Object.fromEntries([...this.#headers].map(([key, [, value]]) => [key, value]));
    }

    writeHead(status, message, headers) {
        this.statusCode = status;
        if (typeof message === 'string') {
            this.statusMessage = message;
        } else {
            headers = message;
        }
        const list = Array.isArray(headers) ? headers : Object.entries(headers ?? {});
        for (const [name, value] of list) {
            this.setHeader(name, value);
        }
        return this;
    }

    #start(length) {
        if (this.#started) {
            return;
        }
        this.#started = true;
        this.headersSent = true;
        if (length !== undefined && !this.hasHeader('content-length')) {
            this.setHeader('Content-Length', length);
        }
        // What's written a part at a time, without saying how long it'll be, goes in chunks, as Node sends it, so that
        // whatever's reading knows each part has come.
        if (!this.hasHeader('content-length') && !this.hasHeader('transfer-encoding') && this.statusCode !== 204 && this.statusCode !== 304) {
            this.setHeader('Transfer-Encoding', 'chunked');
            this.#chunked = true;
        }
        // Each connection's for one request, so that ending it says where the answer ends.
        this.setHeader('Connection', 'close');
        const lines = [`HTTP/1.1 ${this.statusCode} ${this.statusMessage ?? STATUS_TEXT[this.statusCode] ?? ''}`];
        for (const [name, value] of this.#headers.values()) {
            for (const one of Array.isArray(value) ? value : [value]) {
                lines.push(`${name}: ${one}`);
            }
        }
        this.#socket.write(lines.join('\r\n') + '\r\n\r\n', 'latin1');
    }

    flushHeaders() {
        this.#start();
    }

    _write(chunk, encoding, done) {
        this.#start();
        // What's written once the other end has gone has nowhere to go, as in Node.
        if (this.#socket.destroyed) {
            done();
            return;
        }
        if (this.#chunked) {
            if (chunk.length === 0) {
                done();
                return;
            }
            const { Buffer } = require('internal/buffer');
            chunk = Buffer.concat([Buffer.from(chunk.length.toString(16) + '\r\n', 'latin1'), chunk, Buffer.from('\r\n', 'latin1')]);
        }
        this.#socket.write(chunk, encoding, done);
    }

    _final(done) {
        this.#start(0);
        if (this.#socket.destroyed) {
            done();
            return;
        }
        if (this.#chunked) {
            this.#socket.write('0\r\n\r\n', 'latin1');
        }
        this.#socket.end(done);
    }

    end(chunk, encoding, done) {
        if (typeof chunk === 'function') {
            done = chunk;
            chunk = undefined;
        } else if (typeof encoding === 'function') {
            done = encoding;
            encoding = undefined;
        }
        if (!this.#started && chunk !== undefined && chunk !== null) {
            const { Buffer } = require('internal/buffer');
            const bytes = typeof chunk === 'string' ? Buffer.from(chunk, encoding) : chunk;
            this.#start(bytes.length);
            return super.end(bytes, undefined, done);
        }
        return super.end(chunk, encoding, done);
    }
}

const STATUS_TEXT = { 200: 'OK', 201: 'Created', 204: 'No Content', 301: 'Moved Permanently', 302: 'Found', 304: 'Not Modified', 400: 'Bad Request', 401: 'Unauthorized', 403: 'Forbidden', 404: 'Not Found', 405: 'Method Not Allowed', 500: 'Internal Server Error', 503: 'Service Unavailable' };

function makeModule(protocol, defaultPort) {
    const agent = new Agent();
    agent.protocol = protocol;
    agent.defaultPort = defaultPort;
    const request = (first, second, third) => {
        let options;
        let listener = third;
        if (typeof first === 'string' || first instanceof URL) {
            const url = new URL(first);
            options = { href: url.href, protocol: url.protocol, hostname: url.hostname, port: url.port, path: url.pathname + url.search };
            if (typeof second === 'function') {
                listener = second;
            } else if (second) {
                const { href, ...kept } = options;
                options = { ...kept, ...second };
            }
        } else {
            options = first;
            listener = second;
        }
        return new ClientRequest(options, protocol, listener);
    };
    return {
        Agent,
        ClientRequest,
        IncomingMessage: Readable,
        STATUS_CODES,
        METHODS,
        globalAgent: agent,
        request,
        get(first, second, third) {
            const made = request(first, second, third);
            made.end();
            return made;
        },
        createServer(...given) {
            return new Server(given[given.length - 1]);
        },
        Server,
    };
}

module.exports = { http: makeModule('http:', 80), https: makeModule('https:', 443) };
