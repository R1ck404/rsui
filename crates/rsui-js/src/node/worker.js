'use strict';
// Code on another thread, with an engine of its own, which only hears what's sent to it, as a copy. `worker_threads` and the web's
// `Worker` are both this. It's only compiled for what asks for it.

const EventEmitter = require('events');
const { Event, EventTarget } = require('internal/event-target');

const info = __fs.workerInfo();
const isMainThread = info === null;

function fileOf(given) {
    const path = require('path');
    let file = given;
    if (typeof given === 'object' && given !== null && given.href !== undefined) {
        file = given.href;
    }
    file = String(file);
    if (file.startsWith('file:')) {
        const url = new URL(file);
        return decodeURIComponent(url.pathname).replace(/^\/([A-Za-z]:)/, '$1').replaceAll('/', '\\');
    }
    if (path.isAbsolute(file)) {
        return file;
    }
    // It's written from where the program is, rather than wherever it happens to be run from.
    const main = __fs.mainFile();
    return main === null ? path.resolve(file) : path.resolve(path.dirname(main), file);
}

/// Hears what comes from the other side, until it's over.
async function hearForever(number, on) {
    for (;;) {
        const got = await __fs.workerReceive(number);
        if (got === null) {
            return;
        }
        on(got);
    }
}

/// A thread with code in it, as `worker_threads` has it.
class Worker extends EventEmitter {
    #number;
    #exited = null;
    #exit = null;

    constructor(file, options = {}) {
        super();
        this.threadId = 0;
        this.#exited = new Promise((resolve) => {
            this.#exit = resolve;
        });
        const hasData = options.workerData !== undefined;
        this.#number = __fs.workerStart(fileOf(file), hasData, options.workerData);
        this.threadId = this.#number;
        hearForever(this.#number, (got) => this.#heard(got));
    }

    #heard([kind, value]) {
        if (kind === 'message') {
            this.emit('message', value);
        } else if (kind === 'online') {
            this.emit('online');
        } else if (kind === 'error') {
            this.emit('error', new Error(value));
        } else if (kind === 'exit') {
            this.emit('exit', value);
            this.#exit(value);
            __fs.workerForget(this.#number);
        }
    }

    postMessage(message) {
        __fs.workerPost(this.#number, message);
    }

    /// It stops once it's finished what it's in the middle of, as nothing can stop code that's running.
    terminate() {
        __fs.workerStop(this.#number);
        return this.#exited;
    }

    async [Symbol.asyncDispose]() {
        await this.terminate();
    }

    ref() {
        return this;
    }

    unref() {
        return this;
    }
}

class MessageEvent extends Event {
    constructor(type, data) {
        super(type);
        this.data = data;
    }
}

/// A thread with code in it, as the web has it.
class WebWorker extends EventTarget {
    #inner;

    constructor(file, options) {
        super();
        this.name = options?.name ?? '';
        this.onmessage = null;
        this.onerror = null;
        this.#inner = new Worker(file);
        this.#inner.on('message', (data) => this.#tell(new MessageEvent('message', data)));
        this.#inner.on('error', (error) => {
            const event = new Event('error');
            event.message = error.message;
            event.error = error;
            this.#tell(event);
        });
    }

    #tell(event) {
        const handler = this['on' + event.type];
        if (typeof handler === 'function') {
            handler.call(this, event);
        }
        this.dispatchEvent(event);
    }

    postMessage(message) {
        this.#inner.postMessage(message);
    }

    terminate() {
        this.#inner.terminate();
    }
}

/// What's on this side of it, inside a worker.
let parentPort = null;

if (!isMainThread) {
    let hearing = false;
    const startHearing = () => {
        if (!hearing) {
            hearing = true;
            hearForever(0, ([, value]) => {
                parentPort.emit('message', value);
                const event = new MessageEvent('message', value);
                globalThis.onmessage?.(event);
                port.dispatchEvent(event);
            });
        }
    };
    parentPort = new EventEmitter();
    const add = parentPort.on.bind(parentPort);
    parentPort.on = parentPort.addListener = (kind, listener) => {
        if (kind === 'message') {
            startHearing();
        }
        return add(kind, listener);
    };
    parentPort.once = (kind, listener) => {
        const once = (...given) => {
            parentPort.off(kind, once);
            listener(...given);
        };
        return parentPort.on(kind, once);
    };
    parentPort.postMessage = (message) => __fs.parentPost(message);
    parentPort.close = () => __fs.workerClose();
    parentPort.ref = parentPort.unref = () => parentPort;
    const port = new EventTarget();
    const addEvent = port.addEventListener.bind(port);
    globalThis.self = globalThis;
    globalThis.postMessage = parentPort.postMessage;
    globalThis.close = parentPort.close;
    globalThis.addEventListener = (kind, listener) => {
        if (kind === 'message') {
            startHearing();
        }
        addEvent(kind, listener);
    };
    globalThis.removeEventListener = port.removeEventListener.bind(port);
    let onmessage = null;
    Object.defineProperty(globalThis, 'onmessage', {
        configurable: true,
        enumerable: true,
        get: () => onmessage,
        set(value) {
            onmessage = value;
            startHearing();
        },
    });
}

module.exports = {
    isMainThread,
    parentPort,
    workerData: isMainThread ? null : info.data,
    threadId: isMainThread ? 0 : info.id,
    Worker,
    WebWorker,
    MessageChannel: globalThis.MessageChannel,
};
