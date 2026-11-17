'use strict';
// `net`, `dns` and `dgram`, over what the system has. It's only compiled for what asks for these.

const EventEmitter = require('events');
const { Duplex } = require('stream');

const toBuffer = (bytes) => {
    const { Buffer } = require('internal/buffer');
    return Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
};

const toBytes = (chunk, encoding) => {
    const { Buffer } = require('internal/buffer');
    const buffer = typeof chunk === 'string' ? Buffer.from(chunk, encoding) : chunk;
    return new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength);
};

const IPV4 = /^(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)$/;
const isIPv4 = (text) => IPV4.test(String(text));
const isIPv6 = (text) => {
    try {
        return String(text).includes(':') && new URL(`http://[${text}]`).hostname !== '';
    } catch {
        return false;
    }
};

/// What can be said to be connecting to, however it was written.
function connectionOptions(given) {
    const [first, second, third] = given;
    let options;
    let done;
    if (typeof first === 'object' && first !== null) {
        options = first;
        done = second;
    } else if (typeof first === 'string' && Number.isNaN(Number(first))) {
        options = { path: first };
        done = second;
    } else {
        options = { port: first };
        if (typeof second === 'string') {
            options.host = second;
            done = third;
        } else {
            done = second;
        }
    }
    return { options, done };
}

class Socket extends Duplex {
    #number = null;
    #timeout = null;
    #timeoutWait = 0;
    #ended = false;
    #failed = false;

    constructor(options = {}) {
        super({ ...options, allowHalfOpen: options.allowHalfOpen === true, autoDestroy: true });
        this.connecting = false;
        this.remoteAddress = undefined;
        this.remotePort = undefined;
        this.remoteFamily = undefined;
        this.localAddress = undefined;
        this.localPort = undefined;
        this.bytesRead = 0;
        this.bytesWritten = 0;
        if (options.__number !== undefined) {
            this.__opened(options.__number, options.__addresses);
        }
    }

    __opened(number, addresses) {
        this.#number = number;
        this.connecting = false;
        if (addresses !== undefined) {
            const [, localAddress, localPort, remoteAddress, remotePort] = addresses;
            if (localAddress !== '') {
                this.localAddress = localAddress;
                this.localPort = Number(localPort);
                this.remoteAddress = remoteAddress;
                this.remotePort = Number(remotePort);
                this.remoteFamily = isIPv6(remoteAddress) ? 'IPv6' : 'IPv4';
            }
        }
        this.#readForever();
    }

    get readyState() {
        if (this.connecting) {
            return 'opening';
        }
        if (this.#number === null) {
            return 'closed';
        }
        return this.readable && this.writable ? 'open' : this.readable ? 'readOnly' : 'writeOnly';
    }

    async #readForever() {
        const number = this.#number;
        try {
            for (;;) {
                const bytes = await __fs.netRead(number);
                this.#touched();
                if (bytes === null) {
                    this.push(null);
                    return;
                }
                this.bytesRead += bytes.byteLength;
                this.push(toBuffer(bytes));
            }
        } catch (error) {
            this.destroy(error);
        }
    }

    connect(...given) {
        const { options, done } = connectionOptions(given);
        if (typeof done === 'function') {
            this.once('connect', done);
        }
        this.connecting = true;
        const opening =
            options.path !== undefined
                ? __fs.pipeConnect(String(options.path))
                : __fs.netConnect(String(options.host ?? 'localhost'), Number(options.port));
        opening.then(
            (addresses) => {
                if (this.destroyed) {
                    __fs.netClose(Number(addresses[0]));
                    return;
                }
                this.__opened(Number(addresses[0]), addresses);
                if (options.noDelay) {
                    this.setNoDelay(true);
                }
                this.emit('connect');
                this.emit('ready');
            },
            (error) => {
                this.connecting = false;
                this.destroy(error);
            },
        );
        return this;
    }

    _read() {}

    _write(chunk, encoding, done) {
        // What's written before it's open waits until it is.
        if (this.connecting) {
            this.once('connect', () => this._write(chunk, encoding, done));
            return;
        }
        if (this.#number === null) {
            done(Object.assign(new Error('This socket has been ended by the other party'), { code: 'EPIPE' }));
            return;
        }
        const bytes = toBytes(chunk, encoding);
        this.bytesWritten += bytes.byteLength;
        this.#touched();
        __fs.netWrite(this.#number, bytes).then(() => done(), done);
    }

    _final(done) {
        if (this.#number !== null && !this.#ended) {
            this.#ended = true;
            __fs.netEnd(this.#number);
        }
        done();
    }

    /// `close` says whether it ended because of an error.
    emit(kind, ...given) {
        if (kind === 'error') {
            this.#failed = true;
        }
        return kind === 'close' ? super.emit('close', this.#failed) : super.emit(kind, ...given);
    }

    _destroy(error, done) {
        if (this.#timeout !== null) {
            clearTimeout(this.#timeout);
            this.#timeout = null;
        }
        if (this.#number !== null) {
            __fs.netClose(this.#number);
            this.#number = null;
        }
        done(error);
    }

    #touched() {
        if (this.#timeoutWait > 0) {
            clearTimeout(this.#timeout);
            this.#timeout = setTimeout(() => this.emit('timeout'), this.#timeoutWait);
        }
    }

    setTimeout(wait, done) {
        if (typeof done === 'function') {
            this.once('timeout', done);
        }
        clearTimeout(this.#timeout);
        this.#timeout = null;
        this.#timeoutWait = wait;
        this.#touched();
        return this;
    }

    setNoDelay(on = true) {
        if (this.#number !== null) {
            __fs.netNoDelay(this.#number, on);
        }
        return this;
    }

    setKeepAlive() {
        return this;
    }

    address() {
        return this.localAddress === undefined ? {} : { address: this.localAddress, port: this.localPort, family: isIPv6(this.localAddress) ? 'IPv6' : 'IPv4' };
    }

    ref() {
        return this;
    }

    unref() {
        return this;
    }
}

function connect(...given) {
    const socket = new Socket(typeof given[0] === 'object' ? given[0] : {});
    return socket.connect(...given);
}

class Server extends EventEmitter {
    #number = null;
    #address = null;
    #open = new Set();
    #closing = false;
    #onClosed = [];

    constructor(options, onConnection) {
        super();
        if (typeof options === 'function') {
            onConnection = options;
        }
        if (onConnection !== undefined) {
            this.on('connection', onConnection);
        }
    }

    get listening() {
        return this.#number !== null;
    }

    listen(...given) {
        const last = given[given.length - 1];
        let done;
        if (typeof last === 'function') {
            done = last;
            given = given.slice(0, -1);
        }
        const [first, second] = given;
        let options;
        if (typeof first === 'object' && first !== null) {
            options = first;
        } else if (typeof first === 'string' && Number.isNaN(Number(first))) {
            options = { path: first };
        } else {
            options = { port: first ?? 0, host: typeof second === 'string' ? second : undefined };
        }
        if (done !== undefined) {
            this.once('listening', done);
        }
        const opening =
            options.path !== undefined
                ? __fs.pipeListen(String(options.path))
                : __fs.netListen(String(options.host ?? '0.0.0.0'), Number(options.port ?? 0));
        opening.then(
            (found) => {
                this.#number = Number(found[0]);
                this.#address =
                    options.path !== undefined
                        ? String(options.path)
                        : { address: found[1], port: Number(found[2]), family: isIPv6(found[1]) ? 'IPv6' : 'IPv4' };
                this.emit('listening');
                this.#acceptForever(this.#number);
            },
            (error) => this.emit('error', error),
        );
        return this;
    }

    async #acceptForever(number) {
        try {
            for (;;) {
                const found = await __fs.netAccept(number);
                if (found === null) {
                    return;
                }
                const socket = new Socket({ __number: Number(found[0]), __addresses: found });
                this.#open.add(socket);
                socket.once('close', () => {
                    this.#open.delete(socket);
                    this.#finishClosing();
                });
                this.emit('connection', socket);
            }
        } catch (error) {
            if (this.#number !== null) {
                this.emit('error', error);
            }
        }
    }

    #finishClosing() {
        if (this.#closing && this.#open.size === 0) {
            this.#closing = false;
            const waiting = this.#onClosed;
            this.#onClosed = [];
            for (const done of waiting) {
                done();
            }
            this.emit('close');
        }
    }

    close(done) {
        if (this.#number === null) {
            if (typeof done === 'function') {
                queueMicrotask(() => done(Object.assign(new Error('Server is not running.'), { code: 'ERR_SERVER_NOT_RUNNING' })));
            }
            return this;
        }
        if (typeof done === 'function') {
            this.#onClosed.push(done);
        }
        __fs.netStopListening(this.#number);
        this.#number = null;
        this.#address = null;
        this.#closing = true;
        queueMicrotask(() => this.#finishClosing());
        return this;
    }

    async [Symbol.asyncDispose]() {
        if (this.#number !== null) {
            await new Promise((resolve, reject) => this.close((error) => (error ? reject(error) : resolve())));
        }
    }

    address() {
        return this.#address;
    }

    getConnections(done) {
        queueMicrotask(() => done(null, this.#open.size));
        return this;
    }

    ref() {
        return this;
    }

    unref() {
        return this;
    }
}

const net = {
    Socket,
    Server,
    Stream: Socket,
    connect,
    createConnection: connect,
    createServer: (options, onConnection) => new Server(options, onConnection),
    isIP: (text) => (isIPv4(text) ? 4 : isIPv6(text) ? 6 : 0),
    isIPv4,
    isIPv6,
};

function lookupAll(host, family) {
    return __fs.dnsLookup(String(host)).then((found) =>
        found
            .map((address) => ({ address, family: isIPv6(address) ? 6 : 4 }))
            .filter((one) => family === undefined || family === 0 || one.family === family),
    );
}

const dnsPromises = {
    async lookup(host, options = {}) {
        const settings = typeof options === 'number' ? { family: options } : options;
        const found = await lookupAll(host, settings.family);
        if (found.length === 0) {
            throw Object.assign(new Error(`getaddrinfo ENOTFOUND ${host}`), { code: 'ENOTFOUND', hostname: host });
        }
        return settings.all ? found : found[0];
    },
    resolve: async (host) => (await lookupAll(host, 4)).map((one) => one.address),
    resolve4: async (host) => (await lookupAll(host, 4)).map((one) => one.address),
    resolve6: async (host) => (await lookupAll(host, 6)).map((one) => one.address),
};

const dns = {
    promises: dnsPromises,
    lookup(host, options, done) {
        if (typeof options === 'function') {
            done = options;
            options = {};
        }
        dnsPromises.lookup(host, options).then(
            (found) => (Array.isArray(found) ? done(null, found) : done(null, found.address, found.family)),
            (error) => done(error),
        );
    },
    resolve(host, done) {
        dnsPromises.resolve(host).then((found) => done(null, found), done);
    },
    resolve4(host, done) {
        dnsPromises.resolve4(host).then((found) => done(null, found), done);
    },
    resolve6(host, done) {
        dnsPromises.resolve6(host).then((found) => done(null, found), done);
    },
};

class DatagramSocket extends EventEmitter {
    #number = null;
    #address = null;
    #opening = null;

    constructor(type, listener) {
        super();
        this.type = typeof type === 'object' ? type.type : type;
        if (listener !== undefined) {
            this.on('message', listener);
        }
    }

    bind(...given) {
        const last = given[given.length - 1];
        if (typeof last === 'function') {
            this.once('listening', last);
            given = given.slice(0, -1);
        }
        const [first, second] = given;
        const options = typeof first === 'object' && first !== null ? first : { port: first, address: second };
        return this.#open(options.address, options.port) && this;
    }

    #open(address, port) {
        const fallback = this.type === 'udp6' ? '::' : '0.0.0.0';
        this.#opening = __fs.udpBind(String(address ?? fallback), Number(port ?? 0)).then(
            (found) => {
                this.#number = Number(found[0]);
                this.#address = { address: found[1], port: Number(found[2]), family: isIPv6(found[1]) ? 'IPv6' : 'IPv4' };
                this.emit('listening');
                this.#hearForever(this.#number);
            },
            (error) => this.emit('error', error),
        );
        return this.#opening;
    }

    async #hearForever(number) {
        const { Buffer } = require('internal/buffer');
        try {
            for (;;) {
                const found = await __fs.udpReceive(number);
                if (found === null) {
                    return;
                }
                const { bytes, address, port } = found;
                this.emit('message', Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength), {
                    address,
                    port,
                    family: isIPv6(address) ? 'IPv6' : 'IPv4',
                    size: bytes.byteLength,
                });
            }
        } catch (error) {
            this.emit('error', error);
        }
    }

    send(message, ...given) {
        const last = given[given.length - 1];
        const done = typeof last === 'function' ? last : undefined;
        if (done !== undefined) {
            given = given.slice(0, -1);
        }
        let port;
        let address;
        let bytes = toBytes(message);
        if (given.length >= 3) {
            bytes = bytes.subarray(given[0], given[0] + given[1]);
            [, , port, address] = given;
        } else {
            [port, address] = given;
        }
        const sending = (this.#opening ?? this.#open()).then(() =>
            __fs.udpSend(this.#number, bytes, String(address ?? 'localhost'), Number(port)),
        );
        sending.then(
            () => done?.(null, bytes.byteLength),
            (error) => {
                if (done !== undefined) {
                    done(error);
                } else {
                    this.emit('error', error);
                }
            },
        );
    }

    address() {
        return this.#address;
    }

    close(done) {
        if (typeof done === 'function') {
            this.once('close', done);
        }
        if (this.#number !== null) {
            __fs.udpClose(this.#number);
            this.#number = null;
        }
        queueMicrotask(() => this.emit('close'));
        return this;
    }

    async [Symbol.asyncDispose]() {
        if (this.#number !== null) {
            await new Promise((resolve) => this.close(resolve));
        }
    }

    ref() {
        return this;
    }

    unref() {
        return this;
    }
}

const dgram = {
    Socket: DatagramSocket,
    createSocket: (type, listener) => new DatagramSocket(type, listener),
};

module.exports = { net, dns, dgram };
