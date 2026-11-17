'use strict';
// `WebSocket`, as the web has it, over what the system has, which looks after TLS and proxies.

const { Event, EventTarget } = require('internal/event-target');

const CONNECTING = 0;
const OPEN = 1;
const CLOSING = 2;
const CLOSED = 3;

class MessageEvent extends Event {
    constructor(type, data, origin) {
        super(type);
        this.data = data;
        this.origin = origin;
    }
}

class CloseEvent extends Event {
    constructor(type, code, reason, wasClean) {
        super(type);
        this.code = code;
        this.reason = reason;
        this.wasClean = wasClean;
    }
}

class WebSocket extends EventTarget {
    static CONNECTING = CONNECTING;
    static OPEN = OPEN;
    static CLOSING = CLOSING;
    static CLOSED = CLOSED;

    #number = null;
    #sending = Promise.resolve();
    #closeWanted = null;

    constructor(url, protocols = []) {
        super();
        const parsed = new URL(String(url));
        if (parsed.protocol !== 'ws:' && parsed.protocol !== 'wss:') {
            throw new DOMException(`The URL's scheme must be either 'ws' or 'wss'. '${parsed.protocol}' is not allowed.`, 'SyntaxError');
        }
        this.url = parsed.href;
        this.readyState = CONNECTING;
        this.protocol = '';
        this.extensions = '';
        this.binaryType = 'blob';
        this.bufferedAmount = 0;
        this.onopen = null;
        this.onmessage = null;
        this.onerror = null;
        this.onclose = null;
        const list = typeof protocols === 'string' ? protocols : protocols.join(', ');
        __fs.wsConnect(this.url, list).then(
            ([number, protocol]) => {
                this.#number = Number(number);
                if (this.#closeWanted !== null) {
                    const [code, reason] = this.#closeWanted;
                    this.readyState = CLOSING;
                    __fs.wsClose(this.#number, code, reason);
                } else {
                    this.readyState = OPEN;
                    this.protocol = protocol;
                    this.#tell(new Event('open'));
                }
                this.#hearForever();
            },
            (error) => {
                this.readyState = CLOSED;
                const failed = new Event('error');
                failed.error = error;
                failed.message = error.message;
                this.#tell(failed);
                this.#tell(new CloseEvent('close', 1006, '', false));
            },
        );
    }

    get CONNECTING() {
        return CONNECTING;
    }

    get OPEN() {
        return OPEN;
    }

    get CLOSING() {
        return CLOSING;
    }

    get CLOSED() {
        return CLOSED;
    }

    #tell(event) {
        const handler = this['on' + event.type];
        if (typeof handler === 'function') {
            handler.call(this, event);
        }
        this.dispatchEvent(event);
    }

    async #hearForever() {
        const number = this.#number;
        for (;;) {
            const got = await __fs.wsReceive(number);
            if (Array.isArray(got)) {
                const code = Number(got[1]);
                this.readyState = CLOSED;
                __fs.wsForget(number);
                this.#number = null;
                this.#tell(new CloseEvent('close', code, got[2], code !== 1006));
                return;
            }
            let data = got;
            if (typeof got !== 'string') {
                const buffer = got.buffer.slice(got.byteOffset, got.byteOffset + got.byteLength);
                data = this.binaryType === 'arraybuffer' ? buffer : new Blob([buffer]);
            }
            this.#tell(new MessageEvent('message', data, new URL(this.url).origin));
        }
    }

    send(data) {
        if (this.readyState === CONNECTING) {
            throw new DOMException("Failed to execute 'send' on 'WebSocket': Still in CONNECTING state.", 'InvalidStateError');
        }
        if (this.readyState !== OPEN) {
            return;
        }
        const number = this.#number;
        const bytes = (async () => {
            if (typeof data === 'string') {
                return data;
            }
            if (data instanceof Blob) {
                return new Uint8Array(await data.arrayBuffer());
            }
            return ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : new Uint8Array(data);
        })();
        // In the order they were asked for, with each having a thread to itself.
        this.#sending = this.#sending.then(async () => __fs.wsSend(number, await bytes)).catch((error) => {
            const failed = new Event('error');
            failed.error = error;
            this.#tell(failed);
        });
    }

    close(code = 1000, reason = '') {
        if (code !== 1000 && (code < 3000 || code > 4999)) {
            throw new DOMException(`Failed to execute 'close' on 'WebSocket': The close code must be either 1000, or between 3000 and 4999. ${code} is neither.`, 'InvalidAccessError');
        }
        if (this.readyState === CLOSING || this.readyState === CLOSED) {
            return;
        }
        if (this.#number === null) {
            this.#closeWanted = [code, String(reason)];
            return;
        }
        this.readyState = CLOSING;
        const number = this.#number;
        this.#sending = this.#sending.then(() => __fs.wsClose(number, code, String(reason)));
    }
}

module.exports = { WebSocket, MessageEvent, CloseEvent };
