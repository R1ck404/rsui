'use strict';
// Node's streams, for what packages do with them: things that make data, things that take it, and things that go between,
// joined up with `pipe` and `pipeline`. It keeps to the rules that matter to what uses it, without tuning how much is held.

const EventEmitter = require('events');

const WAITING = 16 * 1024;

// FIFO removal releases each value immediately. Copy only after at least 1024 removals
// and when half the slots are spent: storage is O(live entries + 1024). Empty queues
// reuse a small array, so one-at-a-time delivery creates no new array for each chunk.
class StreamQueue {
    constructor() {
        this._items = [];
        this._head = 0;
    }

    get length() {
        return this._items.length - this._head;
    }

    push(value) {
        this._items.push(value);
    }

    shift() {
        const value = this._items[this._head];
        this._items[this._head++] = undefined;
        if (this._head === this._items.length) {
            if (this._items.length <= 16) {
                this._items.length = 0;
            } else {
                this._items = [];
            }
            this._head = 0;
        } else if (this._head >= 1024 && this._head * 2 >= this._items.length) {
            this._items = this._items.slice(this._head);
            this._head = 0;
        }
        return value;
    }

    unshift(value) {
        if (this._head > 0) {
            this._items[--this._head] = value;
        } else {
            // Prepending without a spare slot is linear; the hot FIFO paths never do it.
            this._items.unshift(value);
        }
    }

    takeAll() {
        const items = this._head === 0 ? this._items : this._items.slice(this._head);
        this._items = [];
        this._head = 0;
        return items;
    }
}

// These are plain functions, as a lot of what's written for Node calls them as ones, with `Readable.call(this)`. What
// each can do is written as a class, to save on commas, and copied over.
function extend(child, parent, methods) {
    Object.setPrototypeOf(child.prototype, parent.prototype);
    Object.setPrototypeOf(child, parent);
    const found = Object.getOwnPropertyDescriptors(methods.prototype);
    delete found.constructor;
    Object.defineProperties(child.prototype, found);
}

function bytesOf(chunk, encoding) {
    const { Buffer } = require('internal/buffer');
    if (typeof chunk === 'string') {
        return Buffer.from(chunk, encoding ?? 'utf8');
    }
    // As in Node, bytes that aren't a `Buffer` become one, over the same memory.
    return chunk instanceof Uint8Array && !Buffer.isBuffer(chunk) ? Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength) : chunk;
}

function Stream(options = {}) {
    if (!(this instanceof Stream)) {
        return new Stream(options);
    }
    EventEmitter.call(this);
    this.destroyed = false;
    this.errored = null;
    this.closed = false;
    if (options.destroy) {
        this._destroy = options.destroy;
    }
}

extend(Stream, EventEmitter, class {
    _destroy(error, done) {
        done(error);
    }

    destroy(error) {
        if (this.destroyed) {
            return this;
        }
        this.destroyed = true;
        this._destroy(error ?? null, (found) => {
            queueMicrotask(() => {
                if (found) {
                    this.errored = found;
                    this.emit('error', found);
                }
                this.closed = true;
                this.emit('close');
            });
        });
        return this;
    }

    pipe(target, options) {
        return pipeInto(this, target, options);
    }
});

function pipeInto(source, target, options) {
    const onData = (chunk) => {
        if (target.write(chunk) === false && source.pause) {
            source.pause();
            target.once('drain', () => source.resume());
        }
    };
    source.on('data', onData);
    if (options?.end !== false) {
        source.once('end', () => target.end());
    }
    target.emit('pipe', source);
    return target;
}

function Readable(options = {}) {
    if (!(this instanceof Readable)) {
        return new Readable(options);
    }
    Stream.call(this, options);
    this.readable = true;
    this.readableEnded = false;
    this.readableFlowing = null;
    this.readableObjectMode = !!(options.objectMode || options.readableObjectMode);
    this.readableHighWaterMark = options.highWaterMark ?? options.readableHighWaterMark ?? (this.readableObjectMode ? 16 : WAITING);
    this._chunks = new StreamQueue();
    this._bufferedLength = 0;
    this._ended = false;
    this._reading = false;
    this._encoding = options.encoding ?? null;
    this._decoder = null;
    // What's written for Node can look in here, though it isn't meant to.
    this._readableState = {
        objectMode: this.readableObjectMode,
        highWaterMark: this.readableHighWaterMark,
        ended: false,
        endEmitted: false,
        flowing: null,
        length: 0,
        errorEmitted: false,
        destroyed: false,
    };
    if (options.read) {
        this._read = options.read;
    }
}

extend(Readable, Stream, class {
    _read() {}

    setEncoding(encoding) {
        this._encoding = encoding;
        return this;
    }

    get readableLength() {
        return this._bufferedLength;
    }

    push(chunk, encoding) {
        this._reading = false;
        if (chunk === null) {
            const tail = this._decoder?.end();
            if (tail) {
                this._chunks.push(tail);
                this._bufferedLength += tail.length;
            }
            this._ended = true;
            this._readableState.ended = true;
        } else if (this.readableObjectMode) {
            this._chunks.push(chunk);
            this._bufferedLength++;
        } else if (chunk.length > 0) {
            const decoded = this._decoded(chunk, encoding);
            this._chunks.push(decoded);
            this._bufferedLength += decoded.length;
        }
        this._later();
        return this.readableLength < this.readableHighWaterMark;
    }

    unshift(chunk) {
        this._chunks.unshift(chunk);
        this._bufferedLength += this.readableObjectMode ? 1 : chunk.length;
    }

    _takeChunk() {
        const chunk = this._chunks.shift();
        this._bufferedLength -= this.readableObjectMode ? 1 : chunk.length;
        return chunk;
    }

    _decoded(chunk, encoding) {
        if (this._encoding === null) {
            return bytesOf(chunk, encoding);
        }
        if (typeof chunk === 'string' && (encoding === undefined || encoding === this._encoding)) {
            return chunk;
        }
        if (this._decoder === null) {
            const { StringDecoder } = require('string_decoder');
            this._decoder = new StringDecoder(this._encoding);
        }
        return this._decoder.write(bytesOf(chunk, encoding));
    }

    // What's next happens on a later turn, so that whatever's being set up gets to finish first.
    _later() {
        if (!this._scheduled) {
            this._scheduled = true;
            queueMicrotask(() => {
                this._scheduled = false;
                this._flow();
            });
        }
    }

    _flow() {
        if (this.destroyed) {
            return;
        }
        if (this.readableFlowing === true) {
            while (this._chunks.length > 0 && this.readableFlowing === true && !this.destroyed) {
                this.emit('data', this._takeChunk());
            }
        } else if (this._chunks.length > 0 || this._ended) {
            this.emit('readable');
        }
        if (this.destroyed) {
            return;
        }
        if (this._ended && this._chunks.length === 0 && !this.readableEnded && this.readableFlowing !== false) {
            this.readableEnded = true;
            this._readableState.endEmitted = true;
            this.readable = false;
            this.emit('end');
            if (this.autoDestroy !== false) {
                if (this._writableState === undefined || this.writableFinished) {
                    this.destroy();
                } else if (!this._halfOpen) {
                    // What goes both ways stops once the other side has, but only after what it's been given to write has gone.
                    this.end();
                }
            }
            return;
        }
        if (!this._ended && !this._reading && this.readableFlowing !== false && this.readableLength < this.readableHighWaterMark) {
            this._reading = true;
            this._read(this.readableHighWaterMark);
        }
    }

    on(name, listener) {
        EventEmitter.prototype.on.call(this, name, listener);
        if (name === 'data' && this.readableFlowing !== false) {
            this.resume();
        } else if (name === 'readable' && this.readableFlowing === null) {
            this.readableFlowing = undefined;
            this._later();
        }
        return this;
    }

    addListener(name, listener) {
        return this.on(name, listener);
    }

    read() {
        if (this._chunks.length === 0) {
            this._later();
            return null;
        }
        if (this.readableObjectMode) {
            const chunk = this._takeChunk();
            this._later();
            return chunk;
        }
        const chunks = this._chunks.takeAll();
        this._bufferedLength = 0;
        this._later();
        if (chunks.length === 1) {
            return chunks[0];
        }
        const { Buffer } = require('internal/buffer');
        return typeof chunks[0] === 'string' ? chunks.join('') : Buffer.concat(chunks);
    }

    resume() {
        this.readableFlowing = true;
        this._readableState.flowing = true;
        this._later();
        return this;
    }

    pause() {
        this.readableFlowing = false;
        this._readableState.flowing = false;
        return this;
    }

    isPaused() {
        return this.readableFlowing === false;
    }

    async *[Symbol.asyncIterator]() {
        let wake = null;
        let finished = false;
        let failure = null;
        const chunks = new StreamQueue();
        const notify = () => {
            if (wake) {
                wake();
                wake = null;
            }
        };
        const onData = (next) => {
            this.pause();
            chunks.push(next);
            notify();
        };
        const onEnd = () => {
            finished = true;
            notify();
        };
        const onError = (error) => {
            failure = error;
            notify();
        };
        this.on('data', onData);
        this.once('end', onEnd);
        this.once('close', onEnd);
        this.once('error', onError);
        try {
            for (;;) {
                if (chunks.length) {
                    yield chunks.shift();
                } else if (failure) {
                    throw failure;
                } else if (finished || this.closed || this.readableEnded) {
                    return;
                } else {
                    this.resume();
                    await new Promise((resolve) => { wake = resolve; });
                }
            }
        } finally {
            this.removeListener('data', onData);
            this.removeListener('end', onEnd);
            this.removeListener('close', onEnd);
            this.removeListener('error', onError);
            if (!finished) this.destroy();
        }
    }
});

Readable.from = function from(iterable, options) {
    const stream = new Readable({ objectMode: true, ...options });
    if (typeof iterable === 'string' || ArrayBuffer.isView(iterable)) {
        stream.push(iterable);
        stream.push(null);
        return stream;
    }
    const iterator = iterable[Symbol.asyncIterator]?.() ?? iterable[Symbol.iterator]();
    stream._read = () => {
        Promise.resolve(iterator.next()).then(
            ({ done, value }) => stream.push(done ? null : value),
            (error) => stream.destroy(error),
        );
    };
    return stream;
};

// What takes data is kept apart from `Writable` so that what goes both ways can be both.
const writableParts = {
    write(chunk, encoding, done) {
        if (typeof encoding === 'function') {
            done = encoding;
            encoding = undefined;
        }
        if (this.writableEnded) {
            const error = new Error('write after end');
            error.code = 'ERR_STREAM_WRITE_AFTER_END';
            queueMicrotask(() => {
                done?.(error);
                this.emit('error', error);
            });
            return false;
        }
        if (chunk === null) {
            throw Object.assign(new TypeError('May not write null values to stream'), { code: 'ERR_STREAM_NULL_VALUES' });
        }
        const given = this.writableObjectMode || this._decodeStrings === false ? chunk : bytesOf(chunk, encoding);
        this._writing.push([given, encoding ?? (typeof chunk === 'string' ? 'utf8' : 'buffer'), done]);
        this._held += this.writableObjectMode ? 1 : given.length;
        this._writeNext();
        const roomLeft = this._held < this.writableHighWaterMark;
        if (!roomLeft) {
            this._needDrain = true;
        }
        return roomLeft;
    },

    _writeNext() {
        if (this._busy || this._writing.length === 0) {
            if (!this._busy && this.writableEnded && !this._finishing) {
                this._finish();
            }
            return;
        }
        this._busy = true;
        const [chunk, encoding, done] = this._writing.shift();
        let turnOver = false;
        this._write(chunk, encoding, (error) => {
            const carryOn = () => {
                this._busy = false;
                this._held -= this.writableObjectMode ? 1 : chunk.length;
                if (error) {
                    done?.(error);
                    this.destroy(error);
                    return;
                }
                done?.(null);
                if (this._needDrain && this._held === 0 && !this.writableEnded) {
                    this._needDrain = false;
                    this.emit('drain');
                }
                this._writeNext();
            };
            if (turnOver) {
                carryOn();
            } else {
                queueMicrotask(carryOn);
            }
        });
        turnOver = true;
    },

    _finish() {
        this._finishing = true;
        this._final((error) => {
            queueMicrotask(() => {
                if (error) {
                    this.destroy(error);
                    return;
                }
                this.writableFinished = true;
                this._writableState.finished = true;
                this.emit('finish');
                if (this.autoDestroy !== false && (!(this instanceof Readable) || this.readableEnded)) {
                    this.destroy();
                }
            });
        });
    },

    _final(done) {
        done();
    },

    end(chunk, encoding, done) {
        if (typeof chunk === 'function') {
            done = chunk;
            chunk = undefined;
        } else if (typeof encoding === 'function') {
            done = encoding;
            encoding = undefined;
        }
        if (chunk !== undefined && chunk !== null) {
            this.write(chunk, encoding);
        }
        if (done) {
            this.once('finish', done);
        }
        if (!this.writableEnded) {
            this.writableEnded = true;
            this._writableState.ended = true;
            this.writable = false;
            this._writeNext();
        }
        return this;
    },

    setDefaultEncoding() {
        return this;
    },

    cork() {},

    uncork() {},
};

function setUpWriting(stream, options) {
    stream.writable = true;
    stream.writableEnded = false;
    stream.writableFinished = false;
    stream.writableObjectMode = !!(options.objectMode || options.writableObjectMode);
    stream.writableHighWaterMark = options.highWaterMark ?? options.writableHighWaterMark ?? (stream.writableObjectMode ? 16 : WAITING);
    stream._writableState = {
        objectMode: stream.writableObjectMode,
        highWaterMark: stream.writableHighWaterMark,
        ended: false,
        finished: false,
        length: 0,
        errorEmitted: false,
        destroyed: false,
    };
    stream._writing = [];
    stream._held = 0;
    stream._busy = false;
    stream._finishing = false;
    stream._needDrain = false;
    stream._decodeStrings = options.decodeStrings !== false;
    if (options.write) {
        stream._write = options.write;
    }
    if (options.final) {
        stream._final = options.final;
    }
}

function Writable(options = {}) {
    if (!(this instanceof Writable) && !(this instanceof Duplex)) {
        return new Writable(options);
    }
    Stream.call(this, options);
    setUpWriting(this, options);
}

extend(Writable, Stream, class {
    _write(chunk, encoding, done) {
        done(new Error('The _write() method is not implemented'));
    }
});
Object.assign(Writable.prototype, writableParts);
// What goes both ways is one of these too, though it can only come from one thing.
Object.defineProperty(Writable, Symbol.hasInstance, {
    value(found) {
        return Function.prototype[Symbol.hasInstance].call(this, found) || (this === Writable && found instanceof Duplex);
    },
});

function Duplex(options = {}) {
    if (!(this instanceof Duplex)) {
        return new Duplex(options);
    }
    Readable.call(this, options);
    setUpWriting(this, options);
    this._halfOpen = options.allowHalfOpen === true;
}

extend(Duplex, Readable, class {
    _write(chunk, encoding, done) {
        done(new Error('The _write() method is not implemented'));
    }
});
Object.assign(Duplex.prototype, writableParts);

function Transform(options = {}) {
    if (!(this instanceof Transform)) {
        return new Transform(options);
    }
    Duplex.call(this, options);
    if (options.transform) {
        this._transform = options.transform;
    }
    if (options.flush) {
        this._flush = options.flush;
    }
}

extend(Transform, Duplex, class {
    _transform(chunk, encoding, done) {
        done(new Error('The _transform() method is not implemented'));
    }

    _write(chunk, encoding, done) {
        this._transform(chunk, encoding, (error, value) => {
            if (value !== undefined && value !== null) {
                this.push(value);
            }
            done(error);
        });
    }

    _final(done) {
        const finished = (error, value) => {
            if (value !== undefined && value !== null) {
                this.push(value);
            }
            this.push(null);
            done(error);
        };
        if (this._flush) {
            this._flush(finished);
        } else {
            finished();
        }
    }
});

function PassThrough(options) {
    if (!(this instanceof PassThrough)) {
        return new PassThrough(options);
    }
    Transform.call(this, options);
}

extend(PassThrough, Transform, class {
    _transform(chunk, encoding, done) {
        done(null, chunk);
    }
});

function finished(stream, options, done) {
    if (typeof options === 'function') {
        done = options;
        options = {};
    }
    options ??= {};
    // What it gives back only stops listening, and `done` is only called once, as in Node.
    let listening = true;
    const cleanup = () => {
        listening = false;
        stream.off('error', onError);
        stream.off('end', onEnd);
        stream.off('finish', onFinish);
        stream.off('close', onClose);
    };
    let called = false;
    const finish = (error) => {
        cleanup();
        if (!called) {
            called = true;
            if (error === undefined) {
                done.call(stream);
            } else {
                done.call(stream, error);
            }
        }
    };
    const reads = options.readable !== false && stream instanceof Readable;
    const writes = options.writable !== false && stream.write !== undefined;
    let [readDone, writeDone] = [!reads || stream.readableEnded, !writes || stream.writableFinished];
    const check = () => {
        if (readDone && writeDone) {
            finish();
        }
    };
    const onError = (error) => finish(error);
    const onEnd = () => {
        readDone = true;
        check();
    };
    const onFinish = () => {
        writeDone = true;
        check();
    };
    const onClose = () => {
        if (!readDone || !writeDone) {
            finish(Object.assign(new Error('Premature close'), { code: 'ERR_STREAM_PREMATURE_CLOSE' }));
        }
    };
    stream.on('error', onError);
    stream.on('end', onEnd);
    stream.on('finish', onFinish);
    stream.on('close', onClose);
    // A stream that's already done is told about on the next tick, so what this gives back is there first.
    if (readDone && writeDone) {
        process.nextTick(() => listening && check());
    }
    return cleanup;
}

function pipeline(...streams) {
    const done = typeof streams[streams.length - 1] === 'function' ? streams.pop() : () => {};
    const list = Array.isArray(streams[0]) ? streams[0] : streams;
    let told = false;
    const tell = (error) => {
        if (!told) {
            told = true;
            if (error) {
                for (const stream of list) {
                    stream.destroy?.(error);
                }
            }
            done(error);
        }
    };
    let left = list.length;
    list.forEach((stream, index) => {
        finished(stream, (error) => {
            left -= 1;
            if (error) {
                tell(error);
            } else if (left === 0) {
                tell();
            }
        });
        if (index > 0) {
            list[index - 1].pipe(stream);
        }
    });
    return list[list.length - 1];
}

const promises = {
    pipeline: (...streams) => new Promise((resolve, reject) => pipeline(...streams, (error) => (error ? reject(error) : resolve()))),
    finished: (stream, options) => new Promise((resolve, reject) => finished(stream, options, (error) => (error ? reject(error) : resolve()))),
};

// Between these and the web's streams, as Node does it. The web's are only compiled if one of these is used.
function webStreams() {
    return require('internal/misc/web_streams');
}

Readable.toWeb = function toWeb(readable, options) {
    const { ReadableStream, CountQueuingStrategy, ByteLengthQueuingStrategy } = webStreams();
    const objectMode = readable.readableObjectMode;
    const highWaterMark = readable.readableHighWaterMark;
    const strategy = options?.strategy ?? (objectMode ? new CountQueuingStrategy({ highWaterMark }) : new ByteLengthQueuingStrategy({ highWaterMark }));
    if (readable.destroyed || readable.readableEnded) {
        const ended = new ReadableStream();
        ended.cancel();
        return ended;
    }
    let controller;
    readable.on('data', (chunk) => {
        if (!objectMode && ArrayBuffer.isView(chunk)) {
            chunk = new Uint8Array(chunk);
        } else if (!objectMode && typeof chunk === 'string') {
            chunk = new TextEncoder().encode(chunk);
        }
        controller.enqueue(chunk);
        if (controller.desiredSize <= 0) {
            readable.pause();
        }
    });
    finished(readable, (error) => {
        try {
            if (error && error.code !== 'ERR_STREAM_PREMATURE_CLOSE') {
                controller.error(error);
            } else if (error) {
                controller.error(Object.assign(new Error('The operation was aborted'), { name: 'AbortError', code: 'ABORT_ERR', cause: error }));
            } else {
                controller.close();
            }
        } catch {}
    });
    return new ReadableStream({
        start(made) {
            controller = made;
        },
        pull() {
            readable.resume();
        },
        cancel(reason) {
            readable.destroy(reason);
        },
    }, strategy);
};

Readable.fromWeb = function fromWeb(stream, options = {}) {
    const { Buffer } = require('internal/buffer');
    const { highWaterMark, encoding, objectMode = false } = options;
    const reader = stream.getReader();
    let closed = false;
    const readable = new Readable({
        objectMode,
        highWaterMark,
        encoding,
        read() {
            reader.read().then(({ value, done }) => {
                if (done) {
                    readable.push(null);
                } else {
                    readable.push(!objectMode && value instanceof Uint8Array ? Buffer.from(value.buffer, value.byteOffset, value.byteLength) : value);
                }
            }, (error) => readable.destroy(error));
        },
        destroy(error, done) {
            if (!closed) {
                reader.cancel(error).then(() => done(error), () => done(error));
                return;
            }
            done(error);
        },
    });
    reader.closed.then(() => {
        closed = true;
    }, (error) => {
        closed = true;
        readable.destroy(error);
    });
    return readable;
};

Writable.toWeb = function toWeb(writable) {
    const { WritableStream, CountQueuingStrategy } = webStreams();
    const highWaterMark = writable.writableHighWaterMark;
    let waiting = null;
    let closing = null;
    writable.on('drain', () => {
        waiting?.();
        waiting = null;
    });
    let controller;
    finished(writable, (error) => {
        if (error) {
            try {
                controller.error(error);
            } catch {}
            closing?.reject(error);
        } else {
            closing?.resolve();
        }
        waiting?.();
        waiting = null;
    });
    return new WritableStream({
        start(made) {
            controller = made;
        },
        write(chunk) {
            if (writable.write(chunk) === false) {
                return new Promise((resolve) => {
                    waiting = resolve;
                });
            }
        },
        abort(reason) {
            writable.destroy(reason);
        },
        close() {
            if (closing === null && !writable.writableEnded) {
                return new Promise((resolve, reject) => {
                    closing = { resolve, reject };
                    writable.end();
                });
            }
        },
    }, writable.writableObjectMode ? new CountQueuingStrategy({ highWaterMark }) : { highWaterMark });
};

Writable.fromWeb = function fromWeb(stream, options = {}) {
    const { Buffer } = require('internal/buffer');
    const { highWaterMark, decodeStrings = true, objectMode = false } = options;
    const writer = stream.getWriter();
    let closed = false;
    const writable = new Writable({
        highWaterMark,
        objectMode,
        decodeStrings,
        write(chunk, encoding, done) {
            if (typeof chunk === 'string' && decodeStrings && !objectMode) {
                chunk = Buffer.from(chunk, encoding);
            }
            writer.ready.then(() => writer.write(chunk)).then(() => done(), done);
        },
        final(done) {
            if (!closed) {
                writer.close().then(() => done(), done);
            }
        },
        destroy(error, done) {
            if (!closed) {
                const ending = error !== null && error !== undefined ? writer.abort(error) : writer.close();
                ending.then(() => done(error), () => done(error));
                return;
            }
            done(error);
        },
    });
    writer.closed.then(() => {
        closed = true;
    }, (error) => {
        closed = true;
        writable.destroy(error);
    });
    return writable;
};

Duplex.toWeb = function toWeb(duplex) {
    return { readable: Readable.toWeb(duplex), writable: Writable.toWeb(duplex) };
};

Duplex.fromWeb = function fromWeb(pair, options = {}) {
    const reading = Readable.fromWeb(pair.readable, options);
    const writing = Writable.fromWeb(pair.writable, options);
    const duplex = new Duplex({
        ...options,
        read() {
            reading.resume();
        },
        write(chunk, encoding, done) {
            writing.write(chunk, encoding, done);
        },
        final(done) {
            writing.end(done);
        },
        destroy(error, done) {
            reading.destroy(error);
            writing.destroy(error);
            done(error);
        },
    });
    reading.on('data', (chunk) => {
        if (!duplex.push(chunk)) {
            reading.pause();
        }
    });
    reading.on('end', () => duplex.push(null));
    reading.on('error', (error) => duplex.destroy(error));
    reading.pause();
    return duplex;
};

Stream.Stream = Stream;
Stream.Readable = Readable;
Stream.Writable = Writable;
Stream.Duplex = Duplex;
Stream.Transform = Transform;
Stream.PassThrough = PassThrough;
Stream.finished = finished;
Stream.pipeline = pipeline;
Stream.promises = promises;
module.exports = Stream;
