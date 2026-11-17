'use strict';
// More of what programs and packages expect to find, which isn't compiled until something looks for one of these names.

const navigator = {
    userAgent: 'rsui',
    language: 'en-US',
    languages: ['en-US'],
    platform: 'Win32',
    onLine: true,
    get hardwareConcurrency() {
        return require('os').availableParallelism();
    },
};

const CODES = {
    IndexSizeError: 1, NotFoundError: 8, NotSupportedError: 9, InvalidStateError: 11, SyntaxError: 12, InvalidAccessError: 15,
    TimeoutError: 23, AbortError: 20, DataCloneError: 25, QuotaExceededError: 22, NetworkError: 19, SecurityError: 18,
};

class DOMException extends Error {
    #name;
    constructor(message = '', name = 'Error') {
        super(String(message));
        this.#name = typeof name === 'string' ? name : name?.name ?? 'Error';
    }
    get name() {
        return this.#name;
    }
    get code() {
        return CODES[this.#name] ?? 0;
    }
}

function bytesFrom(parts) {
    const encoder = new TextEncoder();
    const pieces = parts.map((part) => {
        if (part instanceof Blob) {
            return part.bytes;
        }
        if (part instanceof ArrayBuffer) {
            return new Uint8Array(part);
        }
        if (ArrayBuffer.isView(part)) {
            return new Uint8Array(part.buffer, part.byteOffset, part.byteLength);
        }
        return encoder.encode(String(part));
    });
    const out = new Uint8Array(pieces.reduce((total, piece) => total + piece.length, 0));
    let at = 0;
    for (const piece of pieces) {
        out.set(piece, at);
        at += piece.length;
    }
    return out;
}

class Blob {
    constructor(parts = [], options = {}) {
        Object.defineProperty(this, 'bytes', { value: bytesFrom([...parts]) });
        this.size = this.bytes.length;
        this.type = String(options.type ?? '').toLowerCase();
    }
    async text() {
        return new TextDecoder().decode(this.bytes);
    }
    async arrayBuffer() {
        return this.bytes.slice().buffer;
    }
    // Its bytes, a part at a time, as Node gives them.
    stream() {
        const bytes = this.bytes;
        let at = 0;
        const { ReadableStream } = require('internal/misc/web_streams');
        return new ReadableStream({
            type: 'bytes',
            pull(controller) {
                if (at >= bytes.length) {
                    controller.close();
                    controller.byobRequest?.respond(0);
                    return;
                }
                const end = Math.min(at + 65536, bytes.length);
                controller.enqueue(bytes.slice(at, end));
                at = end;
            },
        });
    }
    slice(start = 0, end = this.size, type = '') {
        const piece = new Blob([], { type });
        const made = this.bytes.slice(start, end);
        Object.defineProperty(piece, 'bytes', { value: made });
        piece.size = made.length;
        return piece;
    }
}

class File extends Blob {
    constructor(parts, name, options = {}) {
        super(parts, options);
        this.name = String(name);
        this.lastModified = options.lastModified ?? Date.now();
    }
}

class MessagePort {
    constructor() {
        this.other = null;
        this.onmessage = null;
        this.listeners = new Set();
    }
    postMessage(message) {
        const copy = structuredClone(message);
        const target = this.other;
        setTimeout(() => {
            const event = { data: copy, target };
            target?.onmessage?.(event);
            for (const listener of target?.listeners ?? []) {
                listener(event);
            }
        }, 0);
    }
    addEventListener(name, listener) {
        if (name === 'message') {
            this.listeners.add(listener);
        }
    }
    removeEventListener(name, listener) {
        this.listeners.delete(listener);
    }
    start() {}
    close() {
        this.other = null;
    }
}

class MessageChannel {
    constructor() {
        this.port1 = new MessagePort();
        this.port2 = new MessagePort();
        this.port1.other = this.port2;
        this.port2.other = this.port1;
    }
}

module.exports = { navigator, DOMException, Blob, File, MessageChannel, MessagePort };
