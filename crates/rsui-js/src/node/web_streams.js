'use strict';
// The web's streams, as the Streams Standard writes them, step for step, so that what's waited for happens in the same
// order as in Node or a browser. It's only compiled the first time a program looks for one of these names, or for
// `stream/web`.
//
// What a program sees is a thin object, and what the standard calls its slots are kept on a plain one behind it, found
// through `inner`. That keeps the standard's own steps as plain functions on plain objects, as it writes them.

const promiseThen = Promise.prototype.then;
// What a program gives is called without going through `call` or `apply`, which a program can change.
const reflectApply = Reflect.apply;
const call = (method, self, ...args) => reflectApply(method, self, args);
const noop = () => {};
const inner = new WeakMap();
const typedArrayName = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), Symbol.toStringTag).get;
const transferBuffer = ArrayBuffer.prototype.transfer;
const isDetached = Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, 'detached').get;
const sliceBuffer = ArrayBuffer.prototype.slice;
const AsyncIteratorPrototype = Object.getPrototypeOf(Object.getPrototypeOf(async function* () {}).prototype);

function resolvedWith(value) {
    return new Promise((resolve) => resolve(value));
}
function rejectedWith(reason) {
    return new Promise((_, reject) => reject(reason));
}
function perform(promise, onFulfilled, onRejected) {
    return call(promiseThen, promise, onFulfilled, onRejected);
}
function upon(promise, onFulfilled, onRejected) {
    perform(perform(promise, onFulfilled, onRejected), undefined, noop);
}
function markHandled(promise) {
    perform(promise, undefined, noop);
}

// A promise with what settles it, and whether it has been, which the standard often needs to know.
class Deferred {
    constructor() {
        this.state = 'pending';
        this.promise = new Promise((resolve, reject) => {
            this.resolveWith = resolve;
            this.rejectWith = reject;
        });
    }
    resolve(value) {
        if (this.state === 'pending') {
            this.state = 'fulfilled';
            this.resolveWith(value);
        }
    }
    reject(reason) {
        if (this.state === 'pending') {
            this.state = 'rejected';
            this.rejectWith(reason);
        }
    }
}
function settledDeferred(value) {
    const made = new Deferred();
    made.resolve(value);
    return made;
}
function rejectedDeferred(reason) {
    const made = new Deferred();
    made.reject(reason);
    markHandled(made.promise);
    return made;
}

function of(value, Kind, what) {
    const found = typeof value === 'object' && value !== null ? inner.get(value) : undefined;
    if (!(found instanceof Kind)) {
        throw new TypeError(`Illegal invocation: this is not a ${what}`);
    }
    return found;
}
// The same, for what gives a promise, which is rejected rather than throwing.
function tryOf(value, Kind) {
    const found = typeof value === 'object' && value !== null ? inner.get(value) : undefined;
    return found instanceof Kind ? found : undefined;
}
function illegal() {
    return rejectedWith(new TypeError('Illegal invocation'));
}
function outer(record, proto) {
    const made = Object.create(proto);
    inner.set(made, record);
    record.outer = made;
    return made;
}

// Calls what a program gave, for a promise of how it went, as WebIDL does for a callback that gives a promise.
function promiseCall(method, self, ...args) {
    try {
        return resolvedWith(reflectApply(method, self, args));
    } catch (error) {
        return rejectedWith(error);
    }
}

function method(dict, name) {
    const found = dict[name];
    if (found !== undefined && typeof found !== 'function') {
        throw new TypeError(`${name} must be a function`);
    }
    return found;
}
function dictionary(value, what) {
    if (value === undefined || value === null) {
        return {};
    }
    if (typeof value !== 'object' && typeof value !== 'function') {
        throw new TypeError(`${what} must be an object`);
    }
    return value;
}
function strategyOf(strategy) {
    const dict = dictionary(strategy, 'The strategy');
    const highWaterMark = dict.highWaterMark === undefined ? undefined : +dict.highWaterMark;
    const size = method(dict, 'size');
    return { highWaterMark, size };
}
function highWaterMarkOf(strategy, fallback) {
    if (strategy.highWaterMark === undefined) {
        return fallback;
    }
    if (Number.isNaN(strategy.highWaterMark) || strategy.highWaterMark < 0) {
        throw new RangeError('The highWaterMark must be a number that is not negative');
    }
    return strategy.highWaterMark;
}
function sizeOf(strategy) {
    const size = strategy.size;
    return size === undefined ? () => 1 : (chunk) => size(chunk);
}
// `[EnforceRange] unsigned long long`.
function enforceRange(value, what) {
    let number = +value;
    if (!Number.isFinite(number)) {
        throw new TypeError(`${what} must be a finite number`);
    }
    number = Math.trunc(number);
    if (number < 0 || number > Number.MAX_SAFE_INTEGER) {
        throw new TypeError(`${what} is out of range`);
    }
    return number;
}

// What's queued, with how big each part counts as, for the controllers that take any kind of chunk.
function resetQueue(container) {
    container.queue = [];
    container.queueTotalSize = 0;
}
function enqueueWithSize(container, value, size) {
    if (typeof size !== 'number' || Number.isNaN(size) || size < 0 || size === Infinity) {
        throw new RangeError('The size of a chunk must be a finite number that is not negative');
    }
    container.queue.push({ value, size });
    container.queueTotalSize += size;
}
function dequeueValue(container) {
    const pair = container.queue.shift();
    container.queueTotalSize -= pair.size;
    if (container.queueTotalSize < 0) {
        container.queueTotalSize = 0;
    }
    return pair.value;
}

function isView(value) {
    return ArrayBuffer.isView(value);
}
function viewBuffer(view) {
    return view.buffer;
}
function transfer(buffer) {
    return call(transferBuffer, buffer);
}
function detached(buffer) {
    return call(isDetached, buffer);
}
function elementSizeOf(view) {
    const name = call(typedArrayName, view);
    return name === undefined ? 1 : globalThis[name].BYTES_PER_ELEMENT;
}
function viewConstructorOf(view) {
    const name = call(typedArrayName, view);
    return name === undefined ? DataView : globalThis[name];
}
function copyBytes(to, toIndex, from, fromIndex, count) {
    new Uint8Array(to, toIndex, count).set(new Uint8Array(from, fromIndex, count));
}
function cloneAsUint8Array(view) {
    return new Uint8Array(call(sliceBuffer, view.buffer, view.byteOffset, view.byteOffset + view.byteLength));
}

// ReadableStream

class RS {
    constructor() {
        this.state = 'readable';
        this.reader = undefined;
        this.storedError = undefined;
        this.disturbed = false;
        this.controller = undefined;
        this.outer = undefined;
    }
}
class DefaultController {}
class ByteController {}
class DefaultReader {}
class BYOBReader {}
class BYOBRequestRecord {}

class ReadableStream {
    constructor(underlyingSource = undefined, strategy = undefined) {
        const given = strategyOf(strategy);
        if (underlyingSource === null) {
            throw new TypeError("The underlying source can't be null");
        }
        if (underlyingSource === undefined) {
            underlyingSource = null;
        }
        const source = dictionary(underlyingSource, 'The underlying source');
        const autoAllocateChunkSize = source.autoAllocateChunkSize === undefined ? undefined : enforceRange(source.autoAllocateChunkSize, 'autoAllocateChunkSize');
        const cancel = method(source, 'cancel');
        const pull = method(source, 'pull');
        const start = method(source, 'start');
        let type = source.type;
        if (type !== undefined) {
            type = String(type);
            if (type !== 'bytes') {
                throw new TypeError(`${type} is not a kind of ReadableStream`);
            }
        }
        const stream = new RS();
        inner.set(this, stream);
        stream.outer = this;
        if (type === 'bytes') {
            if (given.size !== undefined) {
                throw new RangeError('A byte stream can\'t have a size function');
            }
            const highWaterMark = highWaterMarkOf(given, 0);
            if (autoAllocateChunkSize === 0) {
                throw new TypeError('autoAllocateChunkSize must be more than 0');
            }
            const controller = new ByteController();
            outer(controller, ReadableByteStreamController.prototype);
            setUpByteController(stream, controller,
                () => (start === undefined ? undefined : call(start, underlyingSource, controller.outer)),
                pull === undefined ? () => resolvedWith(undefined) : () => promiseCall(pull, underlyingSource, controller.outer),
                cancel === undefined ? () => resolvedWith(undefined) : (reason) => promiseCall(cancel, underlyingSource, reason),
                highWaterMark, autoAllocateChunkSize);
        } else {
            const size = sizeOf(given);
            const highWaterMark = highWaterMarkOf(given, 1);
            const controller = new DefaultController();
            outer(controller, ReadableStreamDefaultController.prototype);
            setUpDefaultController(stream, controller,
                () => (start === undefined ? undefined : call(start, underlyingSource, controller.outer)),
                pull === undefined ? () => resolvedWith(undefined) : () => promiseCall(pull, underlyingSource, controller.outer),
                cancel === undefined ? () => resolvedWith(undefined) : (reason) => promiseCall(cancel, underlyingSource, reason),
                highWaterMark, size);
        }
    }

    static from(asyncIterable) {
        return streamFromIterable(asyncIterable).outer;
    }

    get locked() {
        return isLocked(of(this, RS, 'ReadableStream'));
    }

    cancel(reason = undefined) {
        const stream = tryOf(this, RS);
        if (stream === undefined) {
            return illegal();
        }
        if (isLocked(stream)) {
            return rejectedWith(new TypeError('Cannot cancel a stream that is locked to a reader'));
        }
        return cancelStream(stream, reason);
    }

    getReader(options = undefined) {
        const stream = of(this, RS, 'ReadableStream');
        let mode = dictionary(options, 'The options').mode;
        if (mode === undefined) {
            return acquireDefaultReader(stream).outer;
        }
        mode = String(mode);
        if (mode !== 'byob') {
            throw new TypeError(`${mode} is not a kind of reader`);
        }
        return acquireBYOBReader(stream).outer;
    }

    pipeThrough(transform, options = undefined) {
        const stream = of(this, RS, 'ReadableStream');
        const pair = dictionary(transform, 'The transform');
        const readable = pair.readable;
        if (tryOf(readable, RS) === undefined) {
            throw new TypeError('The transform\'s readable must be a ReadableStream');
        }
        const destination = tryOf(pair.writable, WS);
        if (destination === undefined) {
            throw new TypeError('The transform\'s writable must be a WritableStream');
        }
        const { preventAbort, preventCancel, preventClose, signal } = pipeOptions(options);
        if (isLocked(stream)) {
            throw new TypeError('Cannot pipe a stream that is locked to a reader');
        }
        if (isWritableLocked(destination)) {
            throw new TypeError('Cannot pipe to a stream that is locked to a writer');
        }
        markHandled(pipeTo(stream, destination, preventClose, preventAbort, preventCancel, signal));
        return readable;
    }

    pipeTo(destination, options = undefined) {
        const stream = tryOf(this, RS);
        if (stream === undefined) {
            return illegal();
        }
        const to = tryOf(destination, WS);
        if (to === undefined) {
            return rejectedWith(new TypeError('The destination must be a WritableStream'));
        }
        let given;
        try {
            given = pipeOptions(options);
        } catch (error) {
            return rejectedWith(error);
        }
        if (isLocked(stream)) {
            return rejectedWith(new TypeError('Cannot pipe a stream that is locked to a reader'));
        }
        if (isWritableLocked(to)) {
            return rejectedWith(new TypeError('Cannot pipe to a stream that is locked to a writer'));
        }
        return pipeTo(stream, to, given.preventClose, given.preventAbort, given.preventCancel, given.signal);
    }

    tee() {
        const stream = of(this, RS, 'ReadableStream');
        const branches = stream.controller instanceof ByteController ? byteTee(stream) : defaultTee(stream);
        return [branches[0].outer, branches[1].outer];
    }

    values(options = undefined) {
        const stream = of(this, RS, 'ReadableStream');
        const preventCancel = Boolean(dictionary(options, 'The options').preventCancel);
        const reader = acquireDefaultReader(stream);
        const iterator = Object.create(ReadableStreamAsyncIteratorPrototype);
        inner.set(iterator, { reader, preventCancel, ongoing: undefined, finished: false });
        return iterator;
    }
}
ReadableStream.prototype[Symbol.asyncIterator] = ReadableStream.prototype.values;

function pipeOptions(options) {
    const dict = dictionary(options, 'The options');
    const preventAbort = Boolean(dict.preventAbort);
    const preventCancel = Boolean(dict.preventCancel);
    const preventClose = Boolean(dict.preventClose);
    const signal = dict.signal;
    // Only a real signal has its own `aborted`, as that's a field of the class.
    if (signal !== undefined && !(typeof AbortSignal === 'function' && signal instanceof AbortSignal && Object.hasOwn(signal, 'aborted'))) {
        throw new TypeError('The signal must be an AbortSignal');
    }
    return { preventAbort, preventCancel, preventClose, signal };
}

const ReadableStreamAsyncIteratorPrototype = Object.create(AsyncIteratorPrototype, {
    next: {
        writable: true,
        enumerable: true,
        configurable: true,
        value: function next() {
            const state = typeof this === 'object' && this !== null ? inner.get(this) : undefined;
            if (state === undefined || !(state.reader instanceof DefaultReader)) {
                return illegal();
            }
            const nextSteps = () => iteratorNext(state);
            if (state.ongoing === undefined) {
                return nextSteps();
            }
            state.ongoing = perform(state.ongoing, nextSteps, nextSteps);
            return state.ongoing;
        },
    },
    return: {
        writable: true,
        enumerable: true,
        configurable: true,
        value: function return_(value) {
            const state = typeof this === 'object' && this !== null ? inner.get(this) : undefined;
            if (state === undefined || !(state.reader instanceof DefaultReader)) {
                return illegal();
            }
            const returnSteps = () => iteratorReturn(state, value);
            state.ongoing = state.ongoing === undefined ? returnSteps() : perform(state.ongoing, returnSteps, returnSteps);
            return state.ongoing;
        },
    },
});
Object.defineProperty(ReadableStreamAsyncIteratorPrototype.return, 'name', { value: 'return' });

function iteratorNext(state) {
    if (state.finished) {
        return resolvedWith({ value: undefined, done: true });
    }
    const reader = state.reader;
    const promise = new Deferred();
    // What's waited for is only this read until it's been fulfilled, even if that happens right away, so that a
    // `return()` straight after it doesn't wait for anything.
    state.ongoing = promise.promise;
    const fulfilled = () => {
        if (state.ongoing === promise.promise) {
            state.ongoing = undefined;
        }
    };
    readDefault(reader, {
        chunk(chunk) {
            fulfilled();
            queueMicrotask(() => promise.resolve({ value: chunk, done: false }));
        },
        close() {
            fulfilled();
            state.finished = true;
            releaseDefaultReader(reader);
            promise.resolve({ value: undefined, done: true });
        },
        error(error) {
            fulfilled();
            state.finished = true;
            releaseDefaultReader(reader);
            promise.reject(error);
        },
    });
    return promise.promise;
}

function iteratorReturn(state, value) {
    if (state.finished) {
        return resolvedWith({ done: true, value });
    }
    state.finished = true;
    const reader = state.reader;
    if (!state.preventCancel) {
        const result = genericCancel(reader, value);
        releaseDefaultReader(reader);
        return perform(result, () => ({ done: true, value }));
    }
    releaseDefaultReader(reader);
    return resolvedWith({ done: true, value });
}

function streamFromIterable(asyncIterable) {
    let iterator;
    let next;
    const asyncMethod = asyncIterable === null || asyncIterable === undefined ? undefined : asyncIterable[Symbol.asyncIterator];
    if (asyncMethod !== undefined && asyncMethod !== null) {
        iterator = call(asyncMethod, asyncIterable);
        if ((typeof iterator !== 'object' && typeof iterator !== 'function') || iterator === null) {
            throw new TypeError('The async iterator must be an object');
        }
        next = iterator.next;
    } else {
        const syncMethod = asyncIterable === null || asyncIterable === undefined ? undefined : asyncIterable[Symbol.iterator];
        if (typeof syncMethod !== 'function') {
            throw new TypeError('ReadableStream.from needs something that can be iterated');
        }
        const sync = call(syncMethod, asyncIterable);
        if ((typeof sync !== 'object' && typeof sync !== 'function') || sync === null) {
            throw new TypeError('The iterator must be an object');
        }
        const syncNext = sync.next;
        // As the language turns an iterator into an async one, each value is waited for.
        iterator = {
            next() {
                const result = call(syncNext, sync);
                if (typeof result !== 'object' || result === null) {
                    throw new TypeError('An iterator result must be an object');
                }
                const done = Boolean(result.done);
                return perform(resolvedWith(result.value), (value) => ({ value, done }));
            },
            return(value) {
                const found = sync.return;
                if (found === undefined || found === null) {
                    return resolvedWith({ value, done: true });
                }
                const result = call(found, sync, value);
                if (typeof result !== 'object' || result === null) {
                    throw new TypeError('An iterator result must be an object');
                }
                const done = Boolean(result.done);
                return perform(resolvedWith(result.value), (value) => ({ value, done }));
            },
        };
        next = iterator.next;
    }
    let stream;
    const pullAlgorithm = () => {
        let result;
        try {
            result = call(next, iterator);
            if (typeof result !== 'object' || result === null) {
                throw new TypeError('An iterator result must be an object');
            }
        } catch (error) {
            return rejectedWith(error);
        }
        return perform(resolvedWith(result), (found) => {
            if (typeof found !== 'object' || found === null) {
                throw new TypeError('An iterator result must be an object');
            }
            if (found.done) {
                defaultControllerClose(stream.controller);
            } else {
                defaultControllerEnqueue(stream.controller, found.value);
            }
        });
    };
    const cancelAlgorithm = (reason) => {
        let returnMethod;
        let result;
        try {
            returnMethod = iterator.return;
            if (returnMethod === undefined || returnMethod === null) {
                return resolvedWith(undefined);
            }
            if (typeof returnMethod !== 'function') {
                throw new TypeError('The iterator\'s return must be a function');
            }
            result = call(returnMethod, iterator, reason);
        } catch (error) {
            return rejectedWith(error);
        }
        return perform(resolvedWith(result), (found) => {
            if (typeof found !== 'object' || found === null) {
                throw new TypeError('An iterator result must be an object');
            }
        });
    };
    stream = createReadableStream(noop, pullAlgorithm, cancelAlgorithm, 0);
    return stream;
}

function createReadableStream(startAlgorithm, pullAlgorithm, cancelAlgorithm, highWaterMark = 1, sizeAlgorithm = () => 1) {
    const stream = new RS();
    outer(stream, ReadableStream.prototype);
    const controller = new DefaultController();
    outer(controller, ReadableStreamDefaultController.prototype);
    setUpDefaultController(stream, controller, startAlgorithm, pullAlgorithm, cancelAlgorithm, highWaterMark, sizeAlgorithm);
    return stream;
}

function createByteStream(startAlgorithm, pullAlgorithm, cancelAlgorithm) {
    const stream = new RS();
    outer(stream, ReadableStream.prototype);
    const controller = new ByteController();
    outer(controller, ReadableByteStreamController.prototype);
    setUpByteController(stream, controller, startAlgorithm, pullAlgorithm, cancelAlgorithm, 0, undefined);
    return stream;
}

function isLocked(stream) {
    return stream.reader !== undefined;
}

function cancelStream(stream, reason) {
    stream.disturbed = true;
    if (stream.state === 'closed') {
        return resolvedWith(undefined);
    }
    if (stream.state === 'errored') {
        return rejectedWith(stream.storedError);
    }
    closeStream(stream);
    const reader = stream.reader;
    if (reader instanceof BYOBReader) {
        const requests = reader.readIntoRequests;
        reader.readIntoRequests = [];
        for (const request of requests) {
            request.close(undefined);
        }
    }
    const sourceCancelled = stream.controller.cancelSteps(reason);
    return perform(sourceCancelled, noop);
}

function closeStream(stream) {
    stream.state = 'closed';
    const reader = stream.reader;
    if (reader === undefined) {
        return;
    }
    reader.closed.resolve(undefined);
    if (reader instanceof DefaultReader) {
        const requests = reader.readRequests;
        reader.readRequests = [];
        for (const request of requests) {
            request.close();
        }
    }
}

function errorStream(stream, error) {
    stream.state = 'errored';
    stream.storedError = error;
    const reader = stream.reader;
    if (reader === undefined) {
        return;
    }
    reader.closed.reject(error);
    markHandled(reader.closed.promise);
    if (reader instanceof DefaultReader) {
        errorReadRequests(reader, error);
    } else {
        errorReadIntoRequests(reader, error);
    }
}

function fulfillReadRequest(stream, chunk, done) {
    const request = stream.reader.readRequests.shift();
    if (done) {
        request.close();
    } else {
        request.chunk(chunk);
    }
}

function fulfillReadIntoRequest(stream, chunk, done) {
    const request = stream.reader.readIntoRequests.shift();
    if (done) {
        request.close(chunk);
    } else {
        request.chunk(chunk);
    }
}

function readRequestCount(stream) {
    return stream.reader.readRequests.length;
}
function readIntoRequestCount(stream) {
    return stream.reader.readIntoRequests.length;
}
function hasDefaultReader(stream) {
    return stream.reader instanceof DefaultReader;
}
function hasBYOBReader(stream) {
    return stream.reader instanceof BYOBReader;
}

// Readers

function genericInitialize(reader, stream) {
    reader.stream = stream;
    stream.reader = reader;
    if (stream.state === 'readable') {
        reader.closed = new Deferred();
    } else if (stream.state === 'closed') {
        reader.closed = settledDeferred(undefined);
    } else {
        reader.closed = rejectedDeferred(stream.storedError);
    }
}

function genericCancel(reader, reason) {
    return cancelStream(reader.stream, reason);
}

function genericRelease(reader) {
    const stream = reader.stream;
    const released = new TypeError('The reader\'s lock was released');
    if (stream.state === 'readable') {
        reader.closed.reject(released);
    } else {
        reader.closed = new Deferred();
        reader.closed.reject(released);
    }
    markHandled(reader.closed.promise);
    stream.controller.releaseSteps();
    stream.reader = undefined;
    reader.stream = undefined;
}

function releaseDefaultReader(reader) {
    genericRelease(reader);
    errorReadRequests(reader, new TypeError('The reader\'s lock was released'));
}

function releaseBYOBReader(reader) {
    genericRelease(reader);
    errorReadIntoRequests(reader, new TypeError('The reader\'s lock was released'));
}

function errorReadRequests(reader, error) {
    const requests = reader.readRequests;
    reader.readRequests = [];
    for (const request of requests) {
        request.error(error);
    }
}

function errorReadIntoRequests(reader, error) {
    const requests = reader.readIntoRequests;
    reader.readIntoRequests = [];
    for (const request of requests) {
        request.error(error);
    }
}

function setUpDefaultReader(reader, stream) {
    if (isLocked(stream)) {
        throw new TypeError('This stream is already locked to a reader');
    }
    genericInitialize(reader, stream);
    reader.readRequests = [];
}

function setUpBYOBReader(reader, stream) {
    if (isLocked(stream)) {
        throw new TypeError('This stream is already locked to a reader');
    }
    if (!(stream.controller instanceof ByteController)) {
        throw new TypeError('Only a byte stream can have a BYOB reader');
    }
    genericInitialize(reader, stream);
    reader.readIntoRequests = [];
}

function acquireDefaultReader(stream) {
    const reader = new DefaultReader();
    setUpDefaultReader(reader, stream);
    outer(reader, ReadableStreamDefaultReader.prototype);
    return reader;
}

function acquireBYOBReader(stream) {
    const reader = new BYOBReader();
    setUpBYOBReader(reader, stream);
    outer(reader, ReadableStreamBYOBReader.prototype);
    return reader;
}

function readDefault(reader, request) {
    const stream = reader.stream;
    stream.disturbed = true;
    if (stream.state === 'closed') {
        request.close();
    } else if (stream.state === 'errored') {
        request.error(stream.storedError);
    } else {
        stream.controller.pullSteps(request);
    }
}

function readBYOB(reader, view, min, request) {
    const stream = reader.stream;
    stream.disturbed = true;
    if (stream.state === 'errored') {
        request.error(stream.storedError);
    } else {
        pullInto(stream.controller, view, min, request);
    }
}

function readRequestFor(promise) {
    return {
        chunk: (chunk) => promise.resolve({ value: chunk, done: false }),
        close: () => promise.resolve({ value: undefined, done: true }),
        error: (error) => promise.reject(error),
    };
}

class ReadableStreamDefaultReader {
    constructor(stream) {
        const found = of(stream, RS, 'ReadableStream');
        const reader = new DefaultReader();
        setUpDefaultReader(reader, found);
        inner.set(this, reader);
        reader.outer = this;
    }

    get closed() {
        const reader = tryOf(this, DefaultReader);
        return reader === undefined ? illegal() : reader.closed.promise;
    }

    cancel(reason = undefined) {
        const reader = tryOf(this, DefaultReader);
        if (reader === undefined) {
            return illegal();
        }
        if (reader.stream === undefined) {
            return rejectedWith(new TypeError('This reader has been released'));
        }
        return genericCancel(reader, reason);
    }

    read() {
        const reader = tryOf(this, DefaultReader);
        if (reader === undefined) {
            return illegal();
        }
        if (reader.stream === undefined) {
            return rejectedWith(new TypeError('This reader has been released'));
        }
        const promise = new Deferred();
        readDefault(reader, readRequestFor(promise));
        return promise.promise;
    }

    releaseLock() {
        const reader = of(this, DefaultReader, 'ReadableStreamDefaultReader');
        if (reader.stream !== undefined) {
            releaseDefaultReader(reader);
        }
    }
}

class ReadableStreamBYOBReader {
    constructor(stream) {
        const found = of(stream, RS, 'ReadableStream');
        const reader = new BYOBReader();
        setUpBYOBReader(reader, found);
        inner.set(this, reader);
        reader.outer = this;
    }

    get closed() {
        const reader = tryOf(this, BYOBReader);
        return reader === undefined ? illegal() : reader.closed.promise;
    }

    cancel(reason = undefined) {
        const reader = tryOf(this, BYOBReader);
        if (reader === undefined) {
            return illegal();
        }
        if (reader.stream === undefined) {
            return rejectedWith(new TypeError('This reader has been released'));
        }
        return genericCancel(reader, reason);
    }

    read(view, options = undefined) {
        const reader = tryOf(this, BYOBReader);
        if (reader === undefined) {
            return illegal();
        }
        if (!isView(view)) {
            return rejectedWith(new TypeError('read() needs an ArrayBuffer view'));
        }
        if (view.byteLength === 0) {
            return rejectedWith(new TypeError('The view must have room in it'));
        }
        if (detached(viewBuffer(view))) {
            return rejectedWith(new TypeError('The view\'s buffer has been detached'));
        }
        if (viewBuffer(view).byteLength === 0) {
            return rejectedWith(new TypeError('The view\'s buffer must have room in it'));
        }
        let min;
        try {
            const given = dictionary(options, 'The options').min;
            min = given === undefined ? 1 : enforceRange(given, 'min');
        } catch (error) {
            return rejectedWith(error);
        }
        if (min === 0) {
            return rejectedWith(new TypeError('min must be more than 0'));
        }
        if (call(typedArrayName, view) !== undefined) {
            if (min > view.length) {
                return rejectedWith(new RangeError('min is more than the view has room for'));
            }
        } else if (min > view.byteLength) {
            return rejectedWith(new RangeError('min is more than the view has room for'));
        }
        if (reader.stream === undefined) {
            return rejectedWith(new TypeError('This reader has been released'));
        }
        const promise = new Deferred();
        readBYOB(reader, view, min, {
            chunk: (chunk) => promise.resolve({ value: chunk, done: false }),
            close: (chunk) => promise.resolve({ value: chunk, done: true }),
            error: (error) => promise.reject(error),
        });
        return promise.promise;
    }

    releaseLock() {
        const reader = of(this, BYOBReader, 'ReadableStreamBYOBReader');
        if (reader.stream !== undefined) {
            releaseBYOBReader(reader);
        }
    }
}

// The default controller

function setUpDefaultController(stream, controller, startAlgorithm, pullAlgorithm, cancelAlgorithm, highWaterMark, sizeAlgorithm) {
    controller.stream = stream;
    resetQueue(controller);
    controller.started = false;
    controller.closeRequested = false;
    controller.pullAgain = false;
    controller.pulling = false;
    controller.strategySizeAlgorithm = sizeAlgorithm;
    controller.strategyHWM = highWaterMark;
    controller.pullAlgorithm = pullAlgorithm;
    controller.cancelAlgorithm = cancelAlgorithm;
    controller.cancelSteps = defaultCancelSteps;
    controller.pullSteps = defaultPullSteps;
    controller.releaseSteps = noop;
    stream.controller = controller;
    const startResult = startAlgorithm();
    upon(resolvedWith(startResult), () => {
        controller.started = true;
        defaultCallPullIfNeeded(controller);
    }, (error) => defaultControllerError(controller, error));
}

function defaultCancelSteps(reason) {
    resetQueue(this);
    const result = this.cancelAlgorithm(reason);
    defaultClearAlgorithms(this);
    return result;
}

function defaultPullSteps(request) {
    const stream = this.stream;
    if (this.queue.length > 0) {
        const chunk = dequeueValue(this);
        if (this.closeRequested && this.queue.length === 0) {
            defaultClearAlgorithms(this);
            closeStream(stream);
        } else {
            defaultCallPullIfNeeded(this);
        }
        request.chunk(chunk);
    } else {
        stream.reader.readRequests.push(request);
        defaultCallPullIfNeeded(this);
    }
}

function defaultCallPullIfNeeded(controller) {
    if (!defaultShouldCallPull(controller)) {
        return;
    }
    if (controller.pulling) {
        controller.pullAgain = true;
        return;
    }
    controller.pulling = true;
    upon(controller.pullAlgorithm(), () => {
        controller.pulling = false;
        if (controller.pullAgain) {
            controller.pullAgain = false;
            defaultCallPullIfNeeded(controller);
        }
    }, (error) => defaultControllerError(controller, error));
}

function defaultShouldCallPull(controller) {
    const stream = controller.stream;
    if (!defaultCanCloseOrEnqueue(controller) || !controller.started) {
        return false;
    }
    if (isLocked(stream) && readRequestCount(stream) > 0) {
        return true;
    }
    return defaultDesiredSize(controller) > 0;
}

function defaultClearAlgorithms(controller) {
    controller.pullAlgorithm = undefined;
    controller.cancelAlgorithm = undefined;
    controller.strategySizeAlgorithm = undefined;
}

function defaultControllerClose(controller) {
    if (!defaultCanCloseOrEnqueue(controller)) {
        return;
    }
    controller.closeRequested = true;
    if (controller.queue.length === 0) {
        defaultClearAlgorithms(controller);
        closeStream(controller.stream);
    }
}

function defaultControllerEnqueue(controller, chunk) {
    if (!defaultCanCloseOrEnqueue(controller)) {
        return;
    }
    const stream = controller.stream;
    if (isLocked(stream) && readRequestCount(stream) > 0) {
        fulfillReadRequest(stream, chunk, false);
    } else {
        let size;
        try {
            size = controller.strategySizeAlgorithm(chunk);
        } catch (error) {
            defaultControllerError(controller, error);
            throw error;
        }
        try {
            enqueueWithSize(controller, chunk, size);
        } catch (error) {
            defaultControllerError(controller, error);
            throw error;
        }
    }
    defaultCallPullIfNeeded(controller);
}

function defaultControllerError(controller, error) {
    const stream = controller.stream;
    if (stream.state !== 'readable') {
        return;
    }
    resetQueue(controller);
    defaultClearAlgorithms(controller);
    errorStream(stream, error);
}

function defaultDesiredSize(controller) {
    const state = controller.stream.state;
    if (state === 'errored') {
        return null;
    }
    if (state === 'closed') {
        return 0;
    }
    return controller.strategyHWM - controller.queueTotalSize;
}

function defaultHasBackpressure(controller) {
    return !defaultShouldCallPull(controller);
}

function defaultCanCloseOrEnqueue(controller) {
    return !controller.closeRequested && controller.stream.state === 'readable';
}

class ReadableStreamDefaultController {
    constructor() {
        throw new TypeError('Illegal constructor');
    }

    get desiredSize() {
        return defaultDesiredSize(of(this, DefaultController, 'ReadableStreamDefaultController'));
    }

    close() {
        const controller = of(this, DefaultController, 'ReadableStreamDefaultController');
        if (!defaultCanCloseOrEnqueue(controller)) {
            throw new TypeError('The stream is not in a state that can be closed');
        }
        defaultControllerClose(controller);
    }

    enqueue(chunk = undefined) {
        const controller = of(this, DefaultController, 'ReadableStreamDefaultController');
        if (!defaultCanCloseOrEnqueue(controller)) {
            throw new TypeError('The stream is not in a state that can be enqueued to');
        }
        defaultControllerEnqueue(controller, chunk);
    }

    error(error = undefined) {
        defaultControllerError(of(this, DefaultController, 'ReadableStreamDefaultController'), error);
    }
}

// The byte controller

function setUpByteController(stream, controller, startAlgorithm, pullAlgorithm, cancelAlgorithm, highWaterMark, autoAllocateChunkSize) {
    controller.stream = stream;
    controller.pullAgain = false;
    controller.pulling = false;
    controller.byobRequest = null;
    resetQueue(controller);
    controller.closeRequested = false;
    controller.started = false;
    controller.strategyHWM = highWaterMark;
    controller.pullAlgorithm = pullAlgorithm;
    controller.cancelAlgorithm = cancelAlgorithm;
    controller.autoAllocateChunkSize = autoAllocateChunkSize;
    controller.pendingPullIntos = [];
    controller.cancelSteps = byteCancelSteps;
    controller.pullSteps = bytePullSteps;
    controller.releaseSteps = byteReleaseSteps;
    stream.controller = controller;
    const startResult = startAlgorithm();
    upon(resolvedWith(startResult), () => {
        controller.started = true;
        byteCallPullIfNeeded(controller);
    }, (error) => byteControllerError(controller, error));
}

function byteCancelSteps(reason) {
    byteClearPendingPullIntos(this);
    resetQueue(this);
    const result = this.cancelAlgorithm(reason);
    byteClearAlgorithms(this);
    return result;
}

function bytePullSteps(request) {
    const stream = this.stream;
    if (this.queueTotalSize > 0) {
        fillReadRequestFromQueue(this, request);
        return;
    }
    const size = this.autoAllocateChunkSize;
    if (size !== undefined) {
        let buffer;
        try {
            buffer = new ArrayBuffer(size);
        } catch (error) {
            request.error(error);
            return;
        }
        this.pendingPullIntos.push({
            buffer, bufferByteLength: size, byteOffset: 0, byteLength: size, bytesFilled: 0, minimumFill: 1, elementSize: 1,
            viewConstructor: Uint8Array, readerType: 'default',
        });
    }
    stream.reader.readRequests.push(request);
    byteCallPullIfNeeded(this);
}

function byteReleaseSteps() {
    if (this.pendingPullIntos.length > 0) {
        const first = this.pendingPullIntos[0];
        first.readerType = 'none';
        this.pendingPullIntos = [first];
    }
}

function byteCallPullIfNeeded(controller) {
    if (!byteShouldCallPull(controller)) {
        return;
    }
    if (controller.pulling) {
        controller.pullAgain = true;
        return;
    }
    controller.pulling = true;
    upon(controller.pullAlgorithm(), () => {
        controller.pulling = false;
        if (controller.pullAgain) {
            controller.pullAgain = false;
            byteCallPullIfNeeded(controller);
        }
    }, (error) => byteControllerError(controller, error));
}

function byteShouldCallPull(controller) {
    const stream = controller.stream;
    if (stream.state !== 'readable' || controller.closeRequested || !controller.started) {
        return false;
    }
    if (hasDefaultReader(stream) && readRequestCount(stream) > 0) {
        return true;
    }
    if (hasBYOBReader(stream) && readIntoRequestCount(stream) > 0) {
        return true;
    }
    return byteDesiredSize(controller) > 0;
}

function byteClearAlgorithms(controller) {
    controller.pullAlgorithm = undefined;
    controller.cancelAlgorithm = undefined;
}

function byteClearPendingPullIntos(controller) {
    byteInvalidateBYOBRequest(controller);
    controller.pendingPullIntos = [];
}

function byteControllerClose(controller) {
    const stream = controller.stream;
    if (controller.closeRequested || stream.state !== 'readable') {
        return;
    }
    if (controller.queueTotalSize > 0) {
        controller.closeRequested = true;
        return;
    }
    if (controller.pendingPullIntos.length > 0) {
        const first = controller.pendingPullIntos[0];
        if (first.bytesFilled % first.elementSize !== 0) {
            const error = new TypeError('Can\'t close a stream partway through an element');
            byteControllerError(controller, error);
            throw error;
        }
    }
    byteClearAlgorithms(controller);
    closeStream(stream);
}

function commitPullIntoDescriptor(stream, descriptor) {
    let done = false;
    if (stream.state === 'closed') {
        done = true;
    }
    const filledView = convertPullIntoDescriptor(descriptor);
    if (descriptor.readerType === 'default') {
        fulfillReadRequest(stream, filledView, done);
    } else {
        fulfillReadIntoRequest(stream, filledView, done);
    }
}

function convertPullIntoDescriptor(descriptor) {
    const buffer = transfer(descriptor.buffer);
    return new descriptor.viewConstructor(buffer, descriptor.byteOffset, descriptor.bytesFilled / descriptor.elementSize);
}

function byteControllerEnqueue(controller, chunk) {
    const stream = controller.stream;
    if (controller.closeRequested || stream.state !== 'readable') {
        return;
    }
    const buffer = viewBuffer(chunk);
    const byteOffset = chunk.byteOffset;
    const byteLength = chunk.byteLength;
    if (detached(buffer)) {
        throw new TypeError('The chunk\'s buffer has been detached');
    }
    const transferred = transfer(buffer);
    if (controller.pendingPullIntos.length > 0) {
        const first = controller.pendingPullIntos[0];
        if (detached(first.buffer)) {
            throw new TypeError('The BYOB request\'s buffer has been detached');
        }
        byteInvalidateBYOBRequest(controller);
        first.buffer = transfer(first.buffer);
        if (first.readerType === 'none') {
            enqueueDetachedPullIntoToQueue(controller, first);
        }
    }
    if (hasDefaultReader(stream)) {
        processReadRequestsUsingQueue(controller);
        if (readRequestCount(stream) === 0) {
            enqueueChunkToQueue(controller, transferred, byteOffset, byteLength);
        } else {
            if (controller.pendingPullIntos.length > 0) {
                shiftPendingPullInto(controller);
            }
            fulfillReadRequest(stream, new Uint8Array(transferred, byteOffset, byteLength), false);
        }
    } else if (hasBYOBReader(stream)) {
        enqueueChunkToQueue(controller, transferred, byteOffset, byteLength);
        for (const filled of processPullIntoDescriptorsUsingQueue(controller)) {
            commitPullIntoDescriptor(stream, filled);
        }
    } else {
        enqueueChunkToQueue(controller, transferred, byteOffset, byteLength);
    }
    byteCallPullIfNeeded(controller);
}

function enqueueChunkToQueue(controller, buffer, byteOffset, byteLength) {
    controller.queue.push({ buffer, byteOffset, byteLength });
    controller.queueTotalSize += byteLength;
}

function enqueueClonedChunkToQueue(controller, buffer, byteOffset, byteLength) {
    let clone;
    try {
        clone = call(sliceBuffer, buffer, byteOffset, byteOffset + byteLength);
    } catch (error) {
        byteControllerError(controller, error);
        throw error;
    }
    enqueueChunkToQueue(controller, clone, 0, byteLength);
}

function enqueueDetachedPullIntoToQueue(controller, descriptor) {
    if (descriptor.bytesFilled > 0) {
        enqueueClonedChunkToQueue(controller, descriptor.buffer, descriptor.byteOffset, descriptor.bytesFilled);
    }
    shiftPendingPullInto(controller);
}

function byteControllerError(controller, error) {
    const stream = controller.stream;
    if (stream.state !== 'readable') {
        return;
    }
    byteClearPendingPullIntos(controller);
    resetQueue(controller);
    byteClearAlgorithms(controller);
    errorStream(stream, error);
}

function fillHeadPullIntoDescriptor(controller, size, descriptor) {
    descriptor.bytesFilled += size;
}

function fillPullIntoDescriptorFromQueue(controller, descriptor) {
    const maxBytesToCopy = Math.min(controller.queueTotalSize, descriptor.byteLength - descriptor.bytesFilled);
    const maxBytesFilled = descriptor.bytesFilled + maxBytesToCopy;
    let remainingToCopy = maxBytesToCopy;
    let ready = false;
    const remainder = maxBytesFilled % descriptor.elementSize;
    const maxAlignedBytes = maxBytesFilled - remainder;
    if (maxAlignedBytes >= descriptor.minimumFill) {
        remainingToCopy = maxAlignedBytes - descriptor.bytesFilled;
        ready = true;
    }
    const queue = controller.queue;
    while (remainingToCopy > 0) {
        const head = queue[0];
        const bytesToCopy = Math.min(remainingToCopy, head.byteLength);
        const destStart = descriptor.byteOffset + descriptor.bytesFilled;
        copyBytes(descriptor.buffer, destStart, head.buffer, head.byteOffset, bytesToCopy);
        if (head.byteLength === bytesToCopy) {
            queue.shift();
        } else {
            head.byteOffset += bytesToCopy;
            head.byteLength -= bytesToCopy;
        }
        controller.queueTotalSize -= bytesToCopy;
        fillHeadPullIntoDescriptor(controller, bytesToCopy, descriptor);
        remainingToCopy -= bytesToCopy;
    }
    return ready;
}

function byteGetBYOBRequest(controller) {
    if (controller.byobRequest === null && controller.pendingPullIntos.length > 0) {
        const first = controller.pendingPullIntos[0];
        const view = new Uint8Array(first.buffer, first.byteOffset + first.bytesFilled, first.byteLength - first.bytesFilled);
        const request = new BYOBRequestRecord();
        request.controller = controller;
        request.view = view;
        outer(request, ReadableStreamBYOBRequest.prototype);
        controller.byobRequest = request;
    }
    return controller.byobRequest;
}

function byteDesiredSize(controller) {
    const state = controller.stream.state;
    if (state === 'errored') {
        return null;
    }
    if (state === 'closed') {
        return 0;
    }
    return controller.strategyHWM - controller.queueTotalSize;
}

function byteHandleQueueDrain(controller) {
    if (controller.queueTotalSize === 0 && controller.closeRequested) {
        byteClearAlgorithms(controller);
        closeStream(controller.stream);
    } else {
        byteCallPullIfNeeded(controller);
    }
}

function byteInvalidateBYOBRequest(controller) {
    if (controller.byobRequest === null) {
        return;
    }
    controller.byobRequest.controller = undefined;
    controller.byobRequest.view = null;
    controller.byobRequest = null;
}

function processPullIntoDescriptorsUsingQueue(controller) {
    const filled = [];
    while (controller.pendingPullIntos.length > 0) {
        if (controller.queueTotalSize === 0) {
            break;
        }
        const descriptor = controller.pendingPullIntos[0];
        if (fillPullIntoDescriptorFromQueue(controller, descriptor)) {
            shiftPendingPullInto(controller);
            filled.push(descriptor);
        }
    }
    return filled;
}

function processReadRequestsUsingQueue(controller) {
    const reader = controller.stream.reader;
    while (reader.readRequests.length > 0) {
        if (controller.queueTotalSize === 0) {
            return;
        }
        fillReadRequestFromQueue(controller, reader.readRequests.shift());
    }
}

function fillReadRequestFromQueue(controller, request) {
    const entry = controller.queue.shift();
    controller.queueTotalSize -= entry.byteLength;
    byteHandleQueueDrain(controller);
    request.chunk(new Uint8Array(entry.buffer, entry.byteOffset, entry.byteLength));
}

function pullInto(controller, view, min, request) {
    const stream = controller.stream;
    const elementSize = elementSizeOf(view);
    const viewConstructor = viewConstructorOf(view);
    const minimumFill = min * elementSize;
    const byteOffset = view.byteOffset;
    const byteLength = view.byteLength;
    let buffer;
    try {
        buffer = transfer(viewBuffer(view));
    } catch (error) {
        request.error(error);
        return;
    }
    const descriptor = {
        buffer, bufferByteLength: buffer.byteLength, byteOffset, byteLength, bytesFilled: 0, minimumFill, elementSize,
        viewConstructor, readerType: 'byob',
    };
    if (controller.pendingPullIntos.length > 0) {
        controller.pendingPullIntos.push(descriptor);
        stream.reader.readIntoRequests.push(request);
        return;
    }
    if (stream.state === 'closed') {
        request.close(new viewConstructor(descriptor.buffer, descriptor.byteOffset, 0));
        return;
    }
    if (controller.queueTotalSize > 0) {
        if (fillPullIntoDescriptorFromQueue(controller, descriptor)) {
            const filledView = convertPullIntoDescriptor(descriptor);
            byteHandleQueueDrain(controller);
            request.chunk(filledView);
            return;
        }
        if (controller.closeRequested) {
            const error = new TypeError('The stream closed partway through an element');
            byteControllerError(controller, error);
            request.error(error);
            return;
        }
    }
    controller.pendingPullIntos.push(descriptor);
    stream.reader.readIntoRequests.push(request);
    byteCallPullIfNeeded(controller);
}

function byteRespond(controller, bytesWritten) {
    const first = controller.pendingPullIntos[0];
    const state = controller.stream.state;
    if (state === 'closed') {
        if (bytesWritten !== 0) {
            throw new TypeError('bytesWritten must be 0 once the stream has closed');
        }
    } else {
        if (bytesWritten === 0) {
            throw new TypeError('bytesWritten must be more than 0 while the stream is open');
        }
        if (first.bytesFilled + bytesWritten > first.byteLength) {
            throw new RangeError('bytesWritten is more than there was room for');
        }
    }
    first.buffer = transfer(first.buffer);
    byteRespondInternal(controller, bytesWritten);
}

function byteRespondInClosedState(controller, first) {
    if (first.readerType === 'none') {
        shiftPendingPullInto(controller);
    }
    const stream = controller.stream;
    if (hasBYOBReader(stream)) {
        const filled = [];
        while (filled.length < readIntoRequestCount(stream)) {
            filled.push(shiftPendingPullInto(controller));
        }
        for (const descriptor of filled) {
            commitPullIntoDescriptor(stream, descriptor);
        }
    }
}

function byteRespondInReadableState(controller, bytesWritten, descriptor) {
    fillHeadPullIntoDescriptor(controller, bytesWritten, descriptor);
    const stream = controller.stream;
    if (descriptor.readerType === 'none') {
        enqueueDetachedPullIntoToQueue(controller, descriptor);
        for (const filled of processPullIntoDescriptorsUsingQueue(controller)) {
            commitPullIntoDescriptor(stream, filled);
        }
        return;
    }
    if (descriptor.bytesFilled < descriptor.minimumFill) {
        return;
    }
    shiftPendingPullInto(controller);
    const remainder = descriptor.bytesFilled % descriptor.elementSize;
    if (remainder > 0) {
        const end = descriptor.byteOffset + descriptor.bytesFilled;
        enqueueClonedChunkToQueue(controller, descriptor.buffer, end - remainder, remainder);
    }
    descriptor.bytesFilled -= remainder;
    const filled = processPullIntoDescriptorsUsingQueue(controller);
    commitPullIntoDescriptor(stream, descriptor);
    for (const one of filled) {
        commitPullIntoDescriptor(stream, one);
    }
}

function byteRespondInternal(controller, bytesWritten) {
    const first = controller.pendingPullIntos[0];
    byteInvalidateBYOBRequest(controller);
    if (controller.stream.state === 'closed') {
        byteRespondInClosedState(controller, first);
    } else {
        byteRespondInReadableState(controller, bytesWritten, first);
    }
    byteCallPullIfNeeded(controller);
}

function byteRespondWithNewView(controller, view) {
    const first = controller.pendingPullIntos[0];
    const state = controller.stream.state;
    if (state === 'closed') {
        if (view.byteLength !== 0) {
            throw new TypeError('The view must be empty once the stream has closed');
        }
    } else if (view.byteLength === 0) {
        throw new TypeError('The view must not be empty while the stream is open');
    }
    if (first.byteOffset + first.bytesFilled !== view.byteOffset) {
        throw new RangeError('The view must start where the request\'s did');
    }
    if (first.bufferByteLength !== viewBuffer(view).byteLength) {
        throw new RangeError('The view\'s buffer must be the same size as the request\'s');
    }
    if (first.bytesFilled + view.byteLength > first.byteLength) {
        throw new RangeError('The view is bigger than the request');
    }
    const viewByteLength = view.byteLength;
    first.buffer = transfer(viewBuffer(view));
    byteRespondInternal(controller, viewByteLength);
}

function shiftPendingPullInto(controller) {
    return controller.pendingPullIntos.shift();
}

class ReadableByteStreamController {
    constructor() {
        throw new TypeError('Illegal constructor');
    }

    get byobRequest() {
        const request = byteGetBYOBRequest(of(this, ByteController, 'ReadableByteStreamController'));
        return request === null ? null : request.outer;
    }

    get desiredSize() {
        return byteDesiredSize(of(this, ByteController, 'ReadableByteStreamController'));
    }

    close() {
        const controller = of(this, ByteController, 'ReadableByteStreamController');
        if (controller.closeRequested) {
            throw new TypeError('The stream is already closing');
        }
        if (controller.stream.state !== 'readable') {
            throw new TypeError('The stream is not in a state that can be closed');
        }
        byteControllerClose(controller);
    }

    enqueue(chunk) {
        const controller = of(this, ByteController, 'ReadableByteStreamController');
        if (!isView(chunk)) {
            throw new TypeError('A byte stream\'s chunk must be an ArrayBuffer view');
        }
        if (chunk.byteLength === 0) {
            throw new TypeError('The chunk must not be empty');
        }
        if (viewBuffer(chunk).byteLength === 0) {
            throw new TypeError('The chunk\'s buffer must not be empty');
        }
        if (controller.closeRequested) {
            throw new TypeError('The stream is already closing');
        }
        if (controller.stream.state !== 'readable') {
            throw new TypeError('The stream is not in a state that can be enqueued to');
        }
        byteControllerEnqueue(controller, chunk);
    }

    error(error = undefined) {
        byteControllerError(of(this, ByteController, 'ReadableByteStreamController'), error);
    }
}

class ReadableStreamBYOBRequest {
    constructor() {
        throw new TypeError('Illegal constructor');
    }

    get view() {
        return of(this, BYOBRequestRecord, 'ReadableStreamBYOBRequest').view;
    }

    respond(bytesWritten) {
        const request = of(this, BYOBRequestRecord, 'ReadableStreamBYOBRequest');
        const written = enforceRange(bytesWritten, 'bytesWritten');
        if (request.controller === undefined) {
            throw new TypeError('This request has already been answered');
        }
        if (detached(viewBuffer(request.view))) {
            throw new TypeError('The view\'s buffer has been detached');
        }
        byteRespond(request.controller, written);
    }

    respondWithNewView(view) {
        const request = of(this, BYOBRequestRecord, 'ReadableStreamBYOBRequest');
        if (!isView(view)) {
            throw new TypeError('respondWithNewView() needs an ArrayBuffer view');
        }
        if (request.controller === undefined) {
            throw new TypeError('This request has already been answered');
        }
        if (detached(viewBuffer(view))) {
            throw new TypeError('The view\'s buffer has been detached');
        }
        byteRespondWithNewView(request.controller, view);
    }
}

// Teeing

function defaultTee(stream) {
    const reader = acquireDefaultReader(stream);
    let reading = false;
    let readAgain = false;
    let canceled1 = false;
    let canceled2 = false;
    let reason1;
    let reason2;
    let branch1;
    let branch2;
    const cancelPromise = new Deferred();
    const pullAlgorithm = () => {
        if (reading) {
            readAgain = true;
            return resolvedWith(undefined);
        }
        reading = true;
        readDefault(reader, {
            chunk(chunk) {
                queueMicrotask(() => {
                    readAgain = false;
                    if (!canceled1) {
                        defaultControllerEnqueue(branch1.controller, chunk);
                    }
                    if (!canceled2) {
                        defaultControllerEnqueue(branch2.controller, chunk);
                    }
                    reading = false;
                    if (readAgain) {
                        pullAlgorithm();
                    }
                });
            },
            close() {
                reading = false;
                if (!canceled1) {
                    defaultControllerClose(branch1.controller);
                }
                if (!canceled2) {
                    defaultControllerClose(branch2.controller);
                }
                if (!canceled1 || !canceled2) {
                    cancelPromise.resolve(undefined);
                }
            },
            error() {
                reading = false;
            },
        });
        return resolvedWith(undefined);
    };
    const cancel1 = (reason) => {
        canceled1 = true;
        reason1 = reason;
        if (canceled2) {
            cancelPromise.resolve(cancelStream(stream, [reason1, reason2]));
        }
        return cancelPromise.promise;
    };
    const cancel2 = (reason) => {
        canceled2 = true;
        reason2 = reason;
        if (canceled1) {
            cancelPromise.resolve(cancelStream(stream, [reason1, reason2]));
        }
        return cancelPromise.promise;
    };
    branch1 = createReadableStream(noop, pullAlgorithm, cancel1);
    branch2 = createReadableStream(noop, pullAlgorithm, cancel2);
    upon(reader.closed.promise, undefined, (error) => {
        defaultControllerError(branch1.controller, error);
        defaultControllerError(branch2.controller, error);
        if (!canceled1 || !canceled2) {
            cancelPromise.resolve(undefined);
        }
    });
    return [branch1, branch2];
}

function byteTee(stream) {
    let reader = acquireDefaultReader(stream);
    let reading = false;
    let readAgainForBranch1 = false;
    let readAgainForBranch2 = false;
    let canceled1 = false;
    let canceled2 = false;
    let reason1;
    let reason2;
    let branch1;
    let branch2;
    const cancelPromise = new Deferred();
    const forwardReaderError = (thisReader) => {
        upon(thisReader.closed.promise, undefined, (error) => {
            if (thisReader !== reader) {
                return;
            }
            byteControllerError(branch1.controller, error);
            byteControllerError(branch2.controller, error);
            if (!canceled1 || !canceled2) {
                cancelPromise.resolve(undefined);
            }
        });
    };
    const afterChunk = () => {
        reading = false;
        if (readAgainForBranch1) {
            pull1();
        } else if (readAgainForBranch2) {
            pull2();
        }
    };
    const failBoth = (error) => {
        byteControllerError(branch1.controller, error);
        byteControllerError(branch2.controller, error);
        cancelPromise.resolve(cancelStream(stream, error));
    };
    const pullWithDefaultReader = () => {
        if (reader instanceof BYOBReader) {
            releaseBYOBReader(reader);
            reader = acquireDefaultReader(stream);
            forwardReaderError(reader);
        }
        readDefault(reader, {
            chunk(chunk) {
                queueMicrotask(() => {
                    readAgainForBranch1 = false;
                    readAgainForBranch2 = false;
                    const chunk1 = chunk;
                    let chunk2 = chunk;
                    if (!canceled1 && !canceled2) {
                        try {
                            chunk2 = cloneAsUint8Array(chunk);
                        } catch (error) {
                            failBoth(error);
                            return;
                        }
                    }
                    if (!canceled1) {
                        byteControllerEnqueue(branch1.controller, chunk1);
                    }
                    if (!canceled2) {
                        byteControllerEnqueue(branch2.controller, chunk2);
                    }
                    afterChunk();
                });
            },
            close() {
                reading = false;
                if (!canceled1) {
                    byteControllerClose(branch1.controller);
                }
                if (!canceled2) {
                    byteControllerClose(branch2.controller);
                }
                if (branch1.controller.pendingPullIntos.length > 0) {
                    byteRespond(branch1.controller, 0);
                }
                if (branch2.controller.pendingPullIntos.length > 0) {
                    byteRespond(branch2.controller, 0);
                }
                if (!canceled1 || !canceled2) {
                    cancelPromise.resolve(undefined);
                }
            },
            error() {
                reading = false;
            },
        });
    };
    const pullWithBYOBReader = (view, forBranch2) => {
        if (reader instanceof DefaultReader) {
            releaseDefaultReader(reader);
            reader = acquireBYOBReader(stream);
            forwardReaderError(reader);
        }
        const byobBranch = forBranch2 ? branch2 : branch1;
        const otherBranch = forBranch2 ? branch1 : branch2;
        readBYOB(reader, view, 1, {
            chunk(chunk) {
                queueMicrotask(() => {
                    readAgainForBranch1 = false;
                    readAgainForBranch2 = false;
                    const byobCanceled = forBranch2 ? canceled2 : canceled1;
                    const otherCanceled = forBranch2 ? canceled1 : canceled2;
                    if (!otherCanceled) {
                        let cloned;
                        try {
                            cloned = cloneAsUint8Array(chunk);
                        } catch (error) {
                            failBoth(error);
                            return;
                        }
                        if (!byobCanceled) {
                            byteRespondWithNewView(byobBranch.controller, chunk);
                        }
                        byteControllerEnqueue(otherBranch.controller, cloned);
                    } else if (!byobCanceled) {
                        byteRespondWithNewView(byobBranch.controller, chunk);
                    }
                    afterChunk();
                });
            },
            close(chunk) {
                reading = false;
                const byobCanceled = forBranch2 ? canceled2 : canceled1;
                const otherCanceled = forBranch2 ? canceled1 : canceled2;
                if (!byobCanceled) {
                    byteControllerClose(byobBranch.controller);
                }
                if (!otherCanceled) {
                    byteControllerClose(otherBranch.controller);
                }
                if (chunk !== undefined) {
                    if (!byobCanceled) {
                        byteRespondWithNewView(byobBranch.controller, chunk);
                    }
                    if (!otherCanceled && otherBranch.controller.pendingPullIntos.length > 0) {
                        byteRespond(otherBranch.controller, 0);
                    }
                }
                if (!byobCanceled || !otherCanceled) {
                    cancelPromise.resolve(undefined);
                }
            },
            error() {
                reading = false;
            },
        });
    };
    const pullFor = (forBranch2) => {
        if (reading) {
            if (forBranch2) {
                readAgainForBranch2 = true;
            } else {
                readAgainForBranch1 = true;
            }
            return resolvedWith(undefined);
        }
        reading = true;
        const request = byteGetBYOBRequest((forBranch2 ? branch2 : branch1).controller);
        if (request === null) {
            pullWithDefaultReader();
        } else {
            pullWithBYOBReader(request.view, forBranch2);
        }
        return resolvedWith(undefined);
    };
    const pull1 = () => pullFor(false);
    const pull2 = () => pullFor(true);
    const cancel1 = (reason) => {
        canceled1 = true;
        reason1 = reason;
        if (canceled2) {
            cancelPromise.resolve(cancelStream(stream, [reason1, reason2]));
        }
        return cancelPromise.promise;
    };
    const cancel2 = (reason) => {
        canceled2 = true;
        reason2 = reason;
        if (canceled1) {
            cancelPromise.resolve(cancelStream(stream, [reason1, reason2]));
        }
        return cancelPromise.promise;
    };
    branch1 = createByteStream(noop, pull1, cancel1);
    branch2 = createByteStream(noop, pull2, cancel2);
    forwardReaderError(reader);
    return [branch1, branch2];
}

// Piping

function pipeTo(source, dest, preventClose, preventAbort, preventCancel, signal) {
    const reader = acquireDefaultReader(source);
    const writer = acquireWriter(dest);
    source.disturbed = true;
    let shuttingDown = false;
    let currentWrite = resolvedWith(undefined);
    const promise = new Deferred();
    let abortAlgorithm;

    const waitForWritesToFinish = () => {
        const oldCurrentWrite = currentWrite;
        return perform(currentWrite, () => (oldCurrentWrite !== currentWrite ? waitForWritesToFinish() : undefined));
    };
    const finalize = (isError, error) => {
        releaseWriter(writer);
        genericRelease(reader);
        if (signal !== undefined) {
            signal.removeEventListener('abort', abortAlgorithm);
        }
        if (isError) {
            promise.reject(error);
        } else {
            promise.resolve(undefined);
        }
    };
    const shutdownWithAnAction = (action, originalIsError, originalError) => {
        if (shuttingDown) {
            return;
        }
        shuttingDown = true;
        const doTheRest = () => {
            upon(action(), () => finalize(originalIsError, originalError), (newError) => finalize(true, newError));
        };
        if (dest.state === 'writable' && !closeQueuedOrInFlight(dest)) {
            upon(waitForWritesToFinish(), doTheRest);
        } else {
            doTheRest();
        }
    };
    const shutdown = (isError, error) => {
        if (shuttingDown) {
            return;
        }
        shuttingDown = true;
        if (dest.state === 'writable' && !closeQueuedOrInFlight(dest)) {
            upon(waitForWritesToFinish(), () => finalize(isError, error));
        } else {
            finalize(isError, error);
        }
    };

    if (signal !== undefined) {
        abortAlgorithm = () => {
            const error = signal.reason;
            const actions = [];
            if (!preventAbort) {
                actions.push(() => (dest.state === 'writable' ? abortWritable(dest, error) : resolvedWith(undefined)));
            }
            if (!preventCancel) {
                actions.push(() => (source.state === 'readable' ? cancelStream(source, error) : resolvedWith(undefined)));
            }
            shutdownWithAnAction(() => Promise.all(actions.map((action) => action())), true, error);
        };
        if (signal.aborted) {
            abortAlgorithm();
            return promise.promise;
        }
        signal.addEventListener('abort', abortAlgorithm);
    }

    const pipeStep = () => {
        if (shuttingDown) {
            return resolvedWith(true);
        }
        return perform(writer.ready.promise, () => {
            const read = new Deferred();
            readDefault(reader, {
                // The write waits for a microtask, so that a chunk being enqueued never runs the sink's write.
                chunk(chunk) {
                    queueMicrotask(() => {
                        currentWrite = perform(writerWrite(writer, chunk), undefined, noop);
                        read.resolve(false);
                    });
                },
                close: () => read.resolve(true),
                error: (error) => read.reject(error),
            });
            return read.promise;
        });
    };
    const pipeLoop = () => {
        const loop = new Deferred();
        const next = (done) => {
            if (done) {
                loop.resolve(undefined);
            } else {
                perform(pipeStep(), next, (error) => loop.reject(error));
            }
        };
        next(false);
        return loop.promise;
    };

    // What goes wrong at either end, and either end closing, is passed on to the other.
    if (source.state === 'errored') {
        sourceErrored(source.storedError);
    } else {
        upon(reader.closed.promise, undefined, sourceErrored);
    }
    function sourceErrored(error) {
        if (!preventAbort) {
            shutdownWithAnAction(() => abortWritable(dest, error), true, error);
        } else {
            shutdown(true, error);
        }
    }
    if (dest.state === 'errored') {
        destErrored(dest.storedError);
    } else {
        upon(writer.closed.promise, undefined, destErrored);
    }
    function destErrored(error) {
        if (!preventCancel) {
            shutdownWithAnAction(() => cancelStream(source, error), true, error);
        } else {
            shutdown(true, error);
        }
    }
    if (source.state === 'closed') {
        sourceClosed();
    } else {
        upon(reader.closed.promise, sourceClosed);
    }
    function sourceClosed() {
        if (!preventClose) {
            shutdownWithAnAction(() => writerCloseWithErrorPropagation(writer));
        } else {
            shutdown();
        }
    }
    if (closeQueuedOrInFlight(dest) || dest.state === 'closed') {
        const destClosed = new TypeError('The destination closed before everything was piped to it');
        if (!preventCancel) {
            shutdownWithAnAction(() => cancelStream(source, destClosed), true, destClosed);
        } else {
            shutdown(true, destClosed);
        }
    }
    markHandled(pipeLoop());
    return promise.promise;
}

// WritableStream

class WS {
    constructor() {
        this.state = 'writable';
        this.storedError = undefined;
        this.writer = undefined;
        this.controller = undefined;
        this.inFlightWriteRequest = undefined;
        this.closeRequest = undefined;
        this.inFlightCloseRequest = undefined;
        this.pendingAbortRequest = undefined;
        this.writeRequests = [];
        this.backpressure = false;
        this.outer = undefined;
    }
}
class Writer {}
class WController {}
const closeSentinel = {};

class WritableStream {
    constructor(underlyingSink = undefined, strategy = undefined) {
        const given = strategyOf(strategy);
        if (underlyingSink === null) {
            throw new TypeError("The underlying sink can't be null");
        }
        if (underlyingSink === undefined) {
            underlyingSink = null;
        }
        const sink = dictionary(underlyingSink, 'The underlying sink');
        const abort = method(sink, 'abort');
        const close = method(sink, 'close');
        const start = method(sink, 'start');
        const type = sink.type;
        const write = method(sink, 'write');
        if (type !== undefined) {
            throw new RangeError('A WritableStream has no types');
        }
        const stream = new WS();
        inner.set(this, stream);
        stream.outer = this;
        const size = sizeOf(given);
        const highWaterMark = highWaterMarkOf(given, 1);
        const controller = new WController();
        outer(controller, WritableStreamDefaultController.prototype);
        setUpWritableController(stream, controller,
            () => (start === undefined ? undefined : call(start, underlyingSink, controller.outer)),
            write === undefined ? () => resolvedWith(undefined) : (chunk) => promiseCall(write, underlyingSink, chunk, controller.outer),
            close === undefined ? () => resolvedWith(undefined) : () => promiseCall(close, underlyingSink),
            abort === undefined ? () => resolvedWith(undefined) : (reason) => promiseCall(abort, underlyingSink, reason),
            highWaterMark, size);
    }

    get locked() {
        return isWritableLocked(of(this, WS, 'WritableStream'));
    }

    abort(reason = undefined) {
        const stream = tryOf(this, WS);
        if (stream === undefined) {
            return illegal();
        }
        if (isWritableLocked(stream)) {
            return rejectedWith(new TypeError('Cannot abort a stream that is locked to a writer'));
        }
        return abortWritable(stream, reason);
    }

    close() {
        const stream = tryOf(this, WS);
        if (stream === undefined) {
            return illegal();
        }
        if (isWritableLocked(stream)) {
            return rejectedWith(new TypeError('Cannot close a stream that is locked to a writer'));
        }
        if (closeQueuedOrInFlight(stream)) {
            return rejectedWith(new TypeError('The stream is already closing'));
        }
        return closeWritable(stream);
    }

    getWriter() {
        return acquireWriter(of(this, WS, 'WritableStream')).outer;
    }
}

function createWritableStream(startAlgorithm, writeAlgorithm, closeAlgorithm, abortAlgorithm, highWaterMark = 1, sizeAlgorithm = () => 1) {
    const stream = new WS();
    outer(stream, WritableStream.prototype);
    const controller = new WController();
    outer(controller, WritableStreamDefaultController.prototype);
    setUpWritableController(stream, controller, startAlgorithm, writeAlgorithm, closeAlgorithm, abortAlgorithm, highWaterMark, sizeAlgorithm);
    return stream;
}

function isWritableLocked(stream) {
    return stream.writer !== undefined;
}

function acquireWriter(stream) {
    const writer = new Writer();
    setUpWriter(writer, stream);
    outer(writer, WritableStreamDefaultWriter.prototype);
    return writer;
}

function abortWritable(stream, reason) {
    if (stream.state === 'closed' || stream.state === 'errored') {
        return resolvedWith(undefined);
    }
    signalAbort(stream.controller, reason);
    const state = stream.state;
    if (state === 'closed' || state === 'errored') {
        return resolvedWith(undefined);
    }
    if (stream.pendingAbortRequest !== undefined) {
        return stream.pendingAbortRequest.promise.promise;
    }
    let wasAlreadyErroring = false;
    if (state === 'erroring') {
        wasAlreadyErroring = true;
        reason = undefined;
    }
    const promise = new Deferred();
    stream.pendingAbortRequest = { promise, reason, wasAlreadyErroring };
    if (!wasAlreadyErroring) {
        startErroring(stream, reason);
    }
    return promise.promise;
}

function closeWritable(stream) {
    const state = stream.state;
    if (state === 'closed' || state === 'errored') {
        return rejectedWith(new TypeError('The stream is already closed or errored'));
    }
    const promise = new Deferred();
    stream.closeRequest = promise;
    const writer = stream.writer;
    if (writer !== undefined && stream.backpressure && state === 'writable') {
        writer.ready.resolve(undefined);
    }
    enqueueWithSize(stream.controller, closeSentinel, 0);
    writableAdvanceQueueIfNeeded(stream.controller);
    return promise.promise;
}

function dealWithRejection(stream, error) {
    if (stream.state === 'writable') {
        startErroring(stream, error);
        return;
    }
    finishErroring(stream);
}

function finishErroring(stream) {
    stream.state = 'errored';
    resetQueue(stream.controller);
    const storedError = stream.storedError;
    for (const request of stream.writeRequests) {
        request.reject(storedError);
    }
    stream.writeRequests = [];
    if (stream.pendingAbortRequest === undefined) {
        rejectCloseAndClosedPromiseIfNeeded(stream);
        return;
    }
    const abortRequest = stream.pendingAbortRequest;
    stream.pendingAbortRequest = undefined;
    if (abortRequest.wasAlreadyErroring) {
        abortRequest.promise.reject(storedError);
        rejectCloseAndClosedPromiseIfNeeded(stream);
        return;
    }
    const controller = stream.controller;
    const result = controller.abortAlgorithm(abortRequest.reason);
    writableClearAlgorithms(controller);
    upon(result, () => {
        abortRequest.promise.resolve(undefined);
        rejectCloseAndClosedPromiseIfNeeded(stream);
    }, (reason) => {
        abortRequest.promise.reject(reason);
        rejectCloseAndClosedPromiseIfNeeded(stream);
    });
}

function finishInFlightClose(stream) {
    stream.inFlightCloseRequest.resolve(undefined);
    stream.inFlightCloseRequest = undefined;
    if (stream.state === 'erroring') {
        stream.storedError = undefined;
        if (stream.pendingAbortRequest !== undefined) {
            stream.pendingAbortRequest.promise.resolve(undefined);
            stream.pendingAbortRequest = undefined;
        }
    }
    stream.state = 'closed';
    const writer = stream.writer;
    if (writer !== undefined) {
        writer.closed.resolve(undefined);
    }
}

function finishInFlightCloseWithError(stream, error) {
    stream.inFlightCloseRequest.reject(error);
    stream.inFlightCloseRequest = undefined;
    if (stream.pendingAbortRequest !== undefined) {
        stream.pendingAbortRequest.promise.reject(error);
        stream.pendingAbortRequest = undefined;
    }
    dealWithRejection(stream, error);
}

function closeQueuedOrInFlight(stream) {
    return stream.closeRequest !== undefined || stream.inFlightCloseRequest !== undefined;
}

function rejectCloseAndClosedPromiseIfNeeded(stream) {
    if (stream.closeRequest !== undefined) {
        stream.closeRequest.reject(stream.storedError);
        stream.closeRequest = undefined;
    }
    const writer = stream.writer;
    if (writer !== undefined) {
        writer.closed.reject(stream.storedError);
        markHandled(writer.closed.promise);
    }
}

function startErroring(stream, reason) {
    const controller = stream.controller;
    stream.state = 'erroring';
    stream.storedError = reason;
    const writer = stream.writer;
    if (writer !== undefined) {
        writerEnsureReadyPromiseRejected(writer, reason);
    }
    if (stream.inFlightWriteRequest === undefined && stream.inFlightCloseRequest === undefined && controller.started) {
        finishErroring(stream);
    }
}

function updateBackpressure(stream, backpressure) {
    const writer = stream.writer;
    if (writer !== undefined && backpressure !== stream.backpressure) {
        if (backpressure) {
            writer.ready = new Deferred();
        } else {
            writer.ready.resolve(undefined);
        }
    }
    stream.backpressure = backpressure;
}

// Writers

function setUpWriter(writer, stream) {
    if (isWritableLocked(stream)) {
        throw new TypeError('This stream is already locked to a writer');
    }
    writer.stream = stream;
    stream.writer = writer;
    const state = stream.state;
    if (state === 'writable') {
        writer.ready = !closeQueuedOrInFlight(stream) && stream.backpressure ? new Deferred() : settledDeferred(undefined);
        writer.closed = new Deferred();
    } else if (state === 'erroring') {
        writer.ready = rejectedDeferred(stream.storedError);
        writer.closed = new Deferred();
    } else if (state === 'closed') {
        writer.ready = settledDeferred(undefined);
        writer.closed = settledDeferred(undefined);
    } else {
        writer.ready = rejectedDeferred(stream.storedError);
        writer.closed = rejectedDeferred(stream.storedError);
    }
}

function writerCloseWithErrorPropagation(writer) {
    const stream = writer.stream;
    const state = stream.state;
    if (closeQueuedOrInFlight(stream) || state === 'closed') {
        return resolvedWith(undefined);
    }
    if (state === 'errored') {
        return rejectedWith(stream.storedError);
    }
    return closeWritable(stream);
}

function writerEnsureClosedPromiseRejected(writer, error) {
    if (writer.closed.state === 'pending') {
        writer.closed.reject(error);
    } else {
        writer.closed = rejectedDeferred(error);
    }
    markHandled(writer.closed.promise);
}

function writerEnsureReadyPromiseRejected(writer, error) {
    if (writer.ready.state === 'pending') {
        writer.ready.reject(error);
    } else {
        writer.ready = rejectedDeferred(error);
    }
    markHandled(writer.ready.promise);
}

function writerDesiredSize(writer) {
    const stream = writer.stream;
    const state = stream.state;
    if (state === 'errored' || state === 'erroring') {
        return null;
    }
    if (state === 'closed') {
        return 0;
    }
    return writableDesiredSize(stream.controller);
}

function releaseWriter(writer) {
    const stream = writer.stream;
    const released = new TypeError('The writer\'s lock was released');
    writerEnsureReadyPromiseRejected(writer, released);
    writerEnsureClosedPromiseRejected(writer, released);
    stream.writer = undefined;
    writer.stream = undefined;
}

function writerWrite(writer, chunk) {
    const stream = writer.stream;
    const controller = stream.controller;
    const chunkSize = writableChunkSize(controller, chunk);
    if (stream !== writer.stream) {
        return rejectedWith(new TypeError('The writer\'s lock was released'));
    }
    const state = stream.state;
    if (state === 'errored') {
        return rejectedWith(stream.storedError);
    }
    if (closeQueuedOrInFlight(stream) || state === 'closed') {
        return rejectedWith(new TypeError('The stream is closing or closed'));
    }
    if (state === 'erroring') {
        return rejectedWith(stream.storedError);
    }
    const promise = new Deferred();
    stream.writeRequests.push(promise);
    writableControllerWrite(controller, chunk, chunkSize);
    return promise.promise;
}

class WritableStreamDefaultWriter {
    constructor(stream) {
        const found = of(stream, WS, 'WritableStream');
        const writer = new Writer();
        setUpWriter(writer, found);
        inner.set(this, writer);
        writer.outer = this;
    }

    get closed() {
        const writer = tryOf(this, Writer);
        return writer === undefined ? illegal() : writer.closed.promise;
    }

    get desiredSize() {
        const writer = of(this, Writer, 'WritableStreamDefaultWriter');
        if (writer.stream === undefined) {
            throw new TypeError('This writer has been released');
        }
        return writerDesiredSize(writer);
    }

    get ready() {
        const writer = tryOf(this, Writer);
        return writer === undefined ? illegal() : writer.ready.promise;
    }

    abort(reason = undefined) {
        const writer = tryOf(this, Writer);
        if (writer === undefined) {
            return illegal();
        }
        if (writer.stream === undefined) {
            return rejectedWith(new TypeError('This writer has been released'));
        }
        return abortWritable(writer.stream, reason);
    }

    close() {
        const writer = tryOf(this, Writer);
        if (writer === undefined) {
            return illegal();
        }
        const stream = writer.stream;
        if (stream === undefined) {
            return rejectedWith(new TypeError('This writer has been released'));
        }
        if (closeQueuedOrInFlight(stream)) {
            return rejectedWith(new TypeError('The stream is already closing'));
        }
        return closeWritable(stream);
    }

    releaseLock() {
        const writer = of(this, Writer, 'WritableStreamDefaultWriter');
        if (writer.stream !== undefined) {
            releaseWriter(writer);
        }
    }

    write(chunk = undefined) {
        const writer = tryOf(this, Writer);
        if (writer === undefined) {
            return illegal();
        }
        if (writer.stream === undefined) {
            return rejectedWith(new TypeError('This writer has been released'));
        }
        return writerWrite(writer, chunk);
    }
}

// The writable controller

function setUpWritableController(stream, controller, startAlgorithm, writeAlgorithm, closeAlgorithm, abortAlgorithm, highWaterMark, sizeAlgorithm) {
    controller.stream = stream;
    stream.controller = controller;
    resetQueue(controller);
    // The signal is only made if something asks for it, as most never do. Until then, this is what it would say.
    controller.abortController = undefined;
    controller.aborted = false;
    controller.abortReason = undefined;
    controller.started = false;
    controller.strategySizeAlgorithm = sizeAlgorithm;
    controller.strategyHWM = highWaterMark;
    controller.writeAlgorithm = writeAlgorithm;
    controller.closeAlgorithm = closeAlgorithm;
    controller.abortAlgorithm = abortAlgorithm;
    updateBackpressure(stream, writableDesiredSize(controller) <= 0);
    const startResult = startAlgorithm();
    upon(resolvedWith(startResult), () => {
        controller.started = true;
        writableAdvanceQueueIfNeeded(controller);
    }, (error) => {
        controller.started = true;
        dealWithRejection(stream, error);
    });
}

function signalAbort(controller, reason) {
    if (controller.aborted) {
        return;
    }
    controller.aborted = true;
    controller.abortReason = reason;
    if (controller.abortController !== undefined) {
        controller.abortController.abort(reason);
    }
}

function writableAdvanceQueueIfNeeded(controller) {
    const stream = controller.stream;
    if (!controller.started || stream.inFlightWriteRequest !== undefined) {
        return;
    }
    if (stream.state === 'erroring') {
        finishErroring(stream);
        return;
    }
    if (controller.queue.length === 0) {
        return;
    }
    const value = controller.queue[0].value;
    if (value === closeSentinel) {
        writableProcessClose(controller);
    } else {
        writableProcessWrite(controller, value);
    }
}

function writableClearAlgorithms(controller) {
    controller.writeAlgorithm = undefined;
    controller.closeAlgorithm = undefined;
    controller.abortAlgorithm = undefined;
    controller.strategySizeAlgorithm = undefined;
}

function writableControllerError(controller, error) {
    writableClearAlgorithms(controller);
    startErroring(controller.stream, error);
}

function writableErrorIfNeeded(controller, error) {
    if (controller.stream.state === 'writable') {
        writableControllerError(controller, error);
    }
}

function writableChunkSize(controller, chunk) {
    if (controller.strategySizeAlgorithm === undefined) {
        return 1;
    }
    try {
        return controller.strategySizeAlgorithm(chunk);
    } catch (error) {
        writableErrorIfNeeded(controller, error);
        return 1;
    }
}

function writableDesiredSize(controller) {
    return controller.strategyHWM - controller.queueTotalSize;
}

function writableProcessClose(controller) {
    const stream = controller.stream;
    stream.inFlightCloseRequest = stream.closeRequest;
    stream.closeRequest = undefined;
    dequeueValue(controller);
    const sinkClosed = controller.closeAlgorithm();
    writableClearAlgorithms(controller);
    upon(sinkClosed, () => finishInFlightClose(stream), (reason) => finishInFlightCloseWithError(stream, reason));
}

function writableProcessWrite(controller, chunk) {
    const stream = controller.stream;
    stream.inFlightWriteRequest = stream.writeRequests.shift();
    upon(controller.writeAlgorithm(chunk), () => {
        stream.inFlightWriteRequest.resolve(undefined);
        stream.inFlightWriteRequest = undefined;
        const state = stream.state;
        dequeueValue(controller);
        if (!closeQueuedOrInFlight(stream) && state === 'writable') {
            updateBackpressure(stream, writableDesiredSize(controller) <= 0);
        }
        writableAdvanceQueueIfNeeded(controller);
    }, (reason) => {
        if (stream.state === 'writable') {
            writableClearAlgorithms(controller);
        }
        stream.inFlightWriteRequest.reject(reason);
        stream.inFlightWriteRequest = undefined;
        dealWithRejection(stream, reason);
    });
}

function writableControllerWrite(controller, chunk, chunkSize) {
    try {
        enqueueWithSize(controller, chunk, chunkSize);
    } catch (error) {
        writableErrorIfNeeded(controller, error);
        return;
    }
    const stream = controller.stream;
    if (!closeQueuedOrInFlight(stream) && stream.state === 'writable') {
        updateBackpressure(stream, writableDesiredSize(controller) <= 0);
    }
    writableAdvanceQueueIfNeeded(controller);
}

class WritableStreamDefaultController {
    constructor() {
        throw new TypeError('Illegal constructor');
    }

    get signal() {
        const controller = of(this, WController, 'WritableStreamDefaultController');
        if (controller.abortController === undefined) {
            controller.abortController = new AbortController();
            if (controller.aborted) {
                controller.abortController.abort(controller.abortReason);
            }
        }
        return controller.abortController.signal;
    }

    error(error = undefined) {
        const controller = of(this, WController, 'WritableStreamDefaultController');
        if (controller.stream.state === 'writable') {
            writableControllerError(controller, error);
        }
    }
}

// TransformStream

class TS {}
class TController {}

class TransformStream {
    constructor(transformer = undefined, writableStrategy = undefined, readableStrategy = undefined) {
        const writableGiven = strategyOf(writableStrategy);
        const readableGiven = strategyOf(readableStrategy);
        if (transformer === null) {
            throw new TypeError("The transformer can't be null");
        }
        if (transformer === undefined) {
            transformer = null;
        }
        const dict = dictionary(transformer, 'The transformer');
        const cancel = method(dict, 'cancel');
        const flush = method(dict, 'flush');
        const readableType = dict.readableType;
        const start = method(dict, 'start');
        const transform = method(dict, 'transform');
        const writableType = dict.writableType;
        if (readableType !== undefined) {
            throw new RangeError('A TransformStream has no readable types');
        }
        if (writableType !== undefined) {
            throw new RangeError('A TransformStream has no writable types');
        }
        const readableHighWaterMark = highWaterMarkOf(readableGiven, 0);
        const readableSize = sizeOf(readableGiven);
        const writableHighWaterMark = highWaterMarkOf(writableGiven, 1);
        const writableSize = sizeOf(writableGiven);
        const stream = new TS();
        inner.set(this, stream);
        stream.outer = this;
        const startPromise = new Deferred();
        initializeTransformStream(stream, startPromise.promise, writableHighWaterMark, writableSize, readableHighWaterMark, readableSize);
        const controller = new TController();
        outer(controller, TransformStreamDefaultController.prototype);
        setUpTransformController(stream, controller,
            transform === undefined
                ? (chunk) => {
                    try {
                        transformEnqueue(controller, chunk);
                        return resolvedWith(undefined);
                    } catch (error) {
                        return rejectedWith(error);
                    }
                }
                : (chunk) => promiseCall(transform, transformer, chunk, controller.outer),
            flush === undefined ? () => resolvedWith(undefined) : () => promiseCall(flush, transformer, controller.outer),
            cancel === undefined ? () => resolvedWith(undefined) : (reason) => promiseCall(cancel, transformer, reason));
        if (start !== undefined) {
            startPromise.resolve(call(start, transformer, controller.outer));
        } else {
            startPromise.resolve(undefined);
        }
    }

    get readable() {
        return of(this, TS, 'TransformStream').readable.outer;
    }

    get writable() {
        return of(this, TS, 'TransformStream').writable.outer;
    }
}

function initializeTransformStream(stream, startPromise, writableHighWaterMark, writableSize, readableHighWaterMark, readableSize) {
    const startAlgorithm = () => startPromise;
    stream.writable = createWritableStream(startAlgorithm,
        (chunk) => transformSinkWrite(stream, chunk),
        () => transformSinkClose(stream),
        (reason) => transformSinkAbort(stream, reason),
        writableHighWaterMark, writableSize);
    stream.readable = createReadableStream(startAlgorithm,
        () => transformSourcePull(stream),
        (reason) => transformSourceCancel(stream, reason),
        readableHighWaterMark, readableSize);
    stream.backpressure = undefined;
    stream.backpressureChangePromise = undefined;
    setTransformBackpressure(stream, true);
    stream.controller = undefined;
}

function transformStreamError(stream, error) {
    defaultControllerError(stream.readable.controller, error);
    errorWritableAndUnblockWrite(stream, error);
}

function errorWritableAndUnblockWrite(stream, error) {
    transformClearAlgorithms(stream.controller);
    writableErrorIfNeeded(stream.writable.controller, error);
    if (stream.backpressure) {
        setTransformBackpressure(stream, false);
    }
}

function setTransformBackpressure(stream, backpressure) {
    if (stream.backpressureChangePromise !== undefined) {
        stream.backpressureChangePromise.resolve(undefined);
    }
    stream.backpressureChangePromise = new Deferred();
    stream.backpressure = backpressure;
}

function setUpTransformController(stream, controller, transformAlgorithm, flushAlgorithm, cancelAlgorithm) {
    controller.stream = stream;
    stream.controller = controller;
    controller.transformAlgorithm = transformAlgorithm;
    controller.flushAlgorithm = flushAlgorithm;
    controller.cancelAlgorithm = cancelAlgorithm;
    controller.finishPromise = undefined;
}

function transformClearAlgorithms(controller) {
    controller.transformAlgorithm = undefined;
    controller.flushAlgorithm = undefined;
    controller.cancelAlgorithm = undefined;
}

function transformEnqueue(controller, chunk) {
    const stream = controller.stream;
    const readableController = stream.readable.controller;
    if (!defaultCanCloseOrEnqueue(readableController)) {
        throw new TypeError('The readable side is not in a state that can be enqueued to');
    }
    try {
        defaultControllerEnqueue(readableController, chunk);
    } catch (error) {
        errorWritableAndUnblockWrite(stream, error);
        throw stream.readable.storedError;
    }
    const backpressure = defaultHasBackpressure(readableController);
    if (backpressure !== stream.backpressure) {
        setTransformBackpressure(stream, true);
    }
}

function transformPerform(controller, chunk) {
    return perform(controller.transformAlgorithm(chunk), undefined, (reason) => {
        transformStreamError(controller.stream, reason);
        throw reason;
    });
}

function transformTerminate(controller) {
    const stream = controller.stream;
    defaultControllerClose(stream.readable.controller);
    errorWritableAndUnblockWrite(stream, new TypeError('The TransformStream was terminated'));
}

function transformSinkWrite(stream, chunk) {
    const controller = stream.controller;
    if (stream.backpressure) {
        return perform(stream.backpressureChangePromise.promise, () => {
            const writable = stream.writable;
            if (writable.state === 'erroring') {
                throw writable.storedError;
            }
            return transformPerform(controller, chunk);
        });
    }
    return transformPerform(controller, chunk);
}

function transformSinkAbort(stream, reason) {
    const controller = stream.controller;
    if (controller.finishPromise !== undefined) {
        return controller.finishPromise.promise;
    }
    const readable = stream.readable;
    const finish = new Deferred();
    controller.finishPromise = finish;
    const cancelled = controller.cancelAlgorithm(reason);
    transformClearAlgorithms(controller);
    upon(cancelled, () => {
        if (readable.state === 'errored') {
            finish.reject(readable.storedError);
        } else {
            defaultControllerError(readable.controller, reason);
            finish.resolve(undefined);
        }
    }, (error) => {
        defaultControllerError(readable.controller, error);
        finish.reject(error);
    });
    return finish.promise;
}

function transformSinkClose(stream) {
    const controller = stream.controller;
    if (controller.finishPromise !== undefined) {
        return controller.finishPromise.promise;
    }
    const readable = stream.readable;
    const finish = new Deferred();
    controller.finishPromise = finish;
    const flushed = controller.flushAlgorithm();
    transformClearAlgorithms(controller);
    upon(flushed, () => {
        if (readable.state === 'errored') {
            finish.reject(readable.storedError);
        } else {
            defaultControllerClose(readable.controller);
            finish.resolve(undefined);
        }
    }, (error) => {
        defaultControllerError(readable.controller, error);
        finish.reject(error);
    });
    return finish.promise;
}

function transformSourceCancel(stream, reason) {
    const controller = stream.controller;
    if (controller.finishPromise !== undefined) {
        return controller.finishPromise.promise;
    }
    const writable = stream.writable;
    const finish = new Deferred();
    controller.finishPromise = finish;
    const cancelled = controller.cancelAlgorithm(reason);
    transformClearAlgorithms(controller);
    upon(cancelled, () => {
        if (writable.state === 'errored') {
            finish.reject(writable.storedError);
        } else {
            writableErrorIfNeeded(writable.controller, reason);
            if (stream.backpressure) {
                setTransformBackpressure(stream, false);
            }
            finish.resolve(undefined);
        }
    }, (error) => {
        writableErrorIfNeeded(writable.controller, error);
        if (stream.backpressure) {
            setTransformBackpressure(stream, false);
        }
        finish.reject(error);
    });
    return finish.promise;
}

function transformSourcePull(stream) {
    setTransformBackpressure(stream, false);
    return stream.backpressureChangePromise.promise;
}

class TransformStreamDefaultController {
    constructor() {
        throw new TypeError('Illegal constructor');
    }

    get desiredSize() {
        return defaultDesiredSize(of(this, TController, 'TransformStreamDefaultController').stream.readable.controller);
    }

    enqueue(chunk = undefined) {
        transformEnqueue(of(this, TController, 'TransformStreamDefaultController'), chunk);
    }

    error(reason = undefined) {
        transformStreamError(of(this, TController, 'TransformStreamDefaultController').stream, reason);
    }

    terminate() {
        transformTerminate(of(this, TController, 'TransformStreamDefaultController'));
    }
}

// Queuing strategies

const byteLengthSize = {
    size(chunk) {
        return chunk.byteLength;
    },
}.size;
const countSize = {
    size() {
        return 1;
    },
}.size;

function strategyHighWaterMark(init, what) {
    if (typeof init !== 'object' || init === null || init.highWaterMark === undefined) {
        throw new TypeError(`${what} needs a highWaterMark`);
    }
    return +init.highWaterMark;
}

class ByteLengthQueuingStrategy {
    #highWaterMark;
    constructor(init) {
        this.#highWaterMark = strategyHighWaterMark(init, 'ByteLengthQueuingStrategy');
    }
    get highWaterMark() {
        return this.#highWaterMark;
    }
    get size() {
        if (!(#highWaterMark in this)) {
            throw new TypeError('Illegal invocation');
        }
        return byteLengthSize;
    }
}

class CountQueuingStrategy {
    #highWaterMark;
    constructor(init) {
        this.#highWaterMark = strategyHighWaterMark(init, 'CountQueuingStrategy');
    }
    get highWaterMark() {
        return this.#highWaterMark;
    }
    get size() {
        if (!(#highWaterMark in this)) {
            throw new TypeError('Illegal invocation');
        }
        return countSize;
    }
}

// Text and compression, which are each a TransformStream with a little more.

function bytesOfChunk(chunk) {
    if (chunk instanceof ArrayBuffer) {
        return new Uint8Array(chunk);
    }
    if (isView(chunk)) {
        return new Uint8Array(viewBuffer(chunk), chunk.byteOffset, chunk.byteLength);
    }
    throw new TypeError('The chunk must be an ArrayBuffer or a view of one');
}

function transformPair(owner, transformer) {
    const made = new TransformStream(transformer);
    inner.set(owner, made);
    return made;
}

class TextEncoderStream {
    constructor() {
        const encoder = new TextEncoder();
        let pending = '';
        transformPair(this, {
            transform(chunk, controller) {
                let text = pending + String(chunk);
                pending = '';
                const last = text.charCodeAt(text.length - 1);
                // Half of a pair might have its other half in the next chunk.
                if (last >= 0xd800 && last <= 0xdbff) {
                    pending = text[text.length - 1];
                    text = text.slice(0, -1);
                }
                if (text.length > 0) {
                    controller.enqueue(encoder.encode(text));
                }
            },
            flush(controller) {
                if (pending !== '') {
                    controller.enqueue(new Uint8Array([0xef, 0xbf, 0xbd]));
                }
            },
        });
    }
    get encoding() {
        of(this, TransformStream, 'TextEncoderStream');
        return 'utf-8';
    }
    get readable() {
        return of(this, TransformStream, 'TextEncoderStream').readable;
    }
    get writable() {
        return of(this, TransformStream, 'TextEncoderStream').writable;
    }
}

const UTF8_LABELS = ['unicode-1-1-utf-8', 'unicode11utf8', 'unicode20utf8', 'utf-8', 'utf8', 'x-unicode20utf8'];

// How many bytes at the end of these are the start of a character whose rest hasn't come yet.
function unfinishedTail(bytes) {
    const length = bytes.length;
    for (let back = 1; back <= 3 && back <= length; back += 1) {
        const byte = bytes[length - back];
        if ((byte & 0xc0) === 0x80) {
            continue;
        }
        const wanted = byte >= 0xf0 ? 4 : byte >= 0xe0 ? 3 : byte >= 0xc0 ? 2 : 1;
        return wanted > back ? back : 0;
    }
    return 0;
}

function countReplacements(text) {
    let count = 0;
    for (let at = text.indexOf('�'); at >= 0; at = text.indexOf('�', at + 1)) {
        count += 1;
    }
    return count;
}
function countEncodedReplacements(bytes) {
    let count = 0;
    for (let at = 0; at + 2 < bytes.length; at += 1) {
        if (bytes[at] === 0xef && bytes[at + 1] === 0xbf && bytes[at + 2] === 0xbd) {
            count += 1;
        }
    }
    return count;
}

class TextDecoderStream {
    #encoding;
    #fatal;
    #ignoreBOM;
    constructor(label = 'utf-8', options = undefined) {
        const name = String(label).trim().toLowerCase();
        if (!UTF8_LABELS.includes(name)) {
            throw new RangeError(`The "${label}" encoding is not supported`);
        }
        const dict = dictionary(options, 'The options');
        this.#encoding = 'utf-8';
        this.#fatal = Boolean(dict.fatal);
        this.#ignoreBOM = Boolean(dict.ignoreBOM);
        const fatal = this.#fatal;
        const ignoreBOM = this.#ignoreBOM;
        const decoder = new TextDecoder();
        let held = new Uint8Array(0);
        let first = true;
        const decode = (bytes, controller) => {
            if (bytes.length === 0) {
                return;
            }
            let text;
            const startsWithMark = bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf;
            // What this engine's decoder does with a mark at the start is only right for the first part.
            if (startsWithMark && (!first || ignoreBOM)) {
                const marked = new Uint8Array(bytes.length + 1);
                marked[0] = 0x41;
                marked.set(bytes, 1);
                text = decoder.decode(marked).slice(1);
            } else {
                text = decoder.decode(bytes);
            }
            first = false;
            if (fatal && countReplacements(text) !== countEncodedReplacements(bytes)) {
                throw new TypeError('The encoded data was not valid utf-8');
            }
            if (text.length > 0) {
                controller.enqueue(text);
            }
        };
        transformPair(this, {
            transform(chunk, controller) {
                const given = bytesOfChunk(chunk);
                let bytes = given;
                if (held.length > 0) {
                    bytes = new Uint8Array(held.length + given.length);
                    bytes.set(held);
                    bytes.set(given, held.length);
                }
                const tail = unfinishedTail(bytes);
                held = bytes.slice(bytes.length - tail);
                decode(bytes.subarray(0, bytes.length - tail), controller);
            },
            flush(controller) {
                if (held.length > 0) {
                    if (fatal) {
                        throw new TypeError('The encoded data was not valid utf-8');
                    }
                    controller.enqueue('�');
                }
            },
        });
    }
    get encoding() {
        return this.#encoding;
    }
    get fatal() {
        return this.#fatal;
    }
    get ignoreBOM() {
        return this.#ignoreBOM;
    }
    get readable() {
        return of(this, TransformStream, 'TextDecoderStream').readable;
    }
    get writable() {
        return of(this, TransformStream, 'TextDecoderStream').writable;
    }
}

// Squeezing and unsqueezing go through `zlib`, so that both kinds of stream do the same.
function zlibTransform(owner, made) {
    let controllerOf;
    let failure = null;
    made.on('data', (chunk) => {
        if (failure === null) {
            controllerOf.enqueue(new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength));
        }
    });
    let ended = null;
    made.once('end', () => ended?.resolve(undefined));
    made.on('error', (error) => {
        failure = error;
        ended?.reject(error);
        try {
            controllerOf.error(error);
        } catch {}
    });
    transformPair(owner, {
        start(controller) {
            controllerOf = controller;
        },
        transform(chunk) {
            const bytes = bytesOfChunk(chunk);
            return new Promise((resolve, reject) => {
                made.write(bytes, (error) => (error ? reject(error) : resolve()));
            });
        },
        flush() {
            if (failure !== null) {
                throw failure;
            }
            ended = new Deferred();
            made.end();
            return ended.promise;
        },
        cancel(reason) {
            made.destroy(reason);
        },
    });
}

// Each format, with what squeezes it and what unsqueezes it. Brotli is only there if `zlib` has it.
const FORMATS = {
    deflate: ['createDeflate', 'createInflate'],
    'deflate-raw': ['createDeflateRaw', 'createInflateRaw'],
    gzip: ['createGzip', 'createGunzip'],
    brotli: ['createBrotliCompress', 'createBrotliDecompress'],
};
function zlibFor(format, which, what) {
    const zlib = require('zlib');
    const name = String(format);
    const maker = Object.hasOwn(FORMATS, name) ? zlib[FORMATS[name][which]] : undefined;
    if (typeof maker !== 'function') {
        throw new TypeError(`${what} doesn't know the format "${name}"`);
    }
    return call(maker, zlib);
}

class CompressionStream {
    constructor(format) {
        zlibTransform(this, zlibFor(format, 0, 'CompressionStream'));
    }
    get readable() {
        return of(this, TransformStream, 'CompressionStream').readable;
    }
    get writable() {
        return of(this, TransformStream, 'CompressionStream').writable;
    }
}

class DecompressionStream {
    constructor(format) {
        zlibTransform(this, zlibFor(format, 1, 'DecompressionStream'));
    }
    get readable() {
        return of(this, TransformStream, 'DecompressionStream').readable;
    }
    get writable() {
        return of(this, TransformStream, 'DecompressionStream').writable;
    }
}

for (const [made, name] of [
    [ReadableStream, 'ReadableStream'], [ReadableStreamDefaultReader, 'ReadableStreamDefaultReader'],
    [ReadableStreamBYOBReader, 'ReadableStreamBYOBReader'], [ReadableStreamDefaultController, 'ReadableStreamDefaultController'],
    [ReadableByteStreamController, 'ReadableByteStreamController'], [ReadableStreamBYOBRequest, 'ReadableStreamBYOBRequest'],
    [WritableStream, 'WritableStream'], [WritableStreamDefaultWriter, 'WritableStreamDefaultWriter'],
    [WritableStreamDefaultController, 'WritableStreamDefaultController'], [TransformStream, 'TransformStream'],
    [TransformStreamDefaultController, 'TransformStreamDefaultController'], [ByteLengthQueuingStrategy, 'ByteLengthQueuingStrategy'],
    [CountQueuingStrategy, 'CountQueuingStrategy'], [TextEncoderStream, 'TextEncoderStream'], [TextDecoderStream, 'TextDecoderStream'],
    [CompressionStream, 'CompressionStream'], [DecompressionStream, 'DecompressionStream'],
]) {
    Object.defineProperty(made.prototype, Symbol.toStringTag, { value: name, configurable: true });
}

// What `fetch` and Node's streams need to know about a stream, which a program can't ask.
const internals = {
    isDisturbed: (stream) => of(stream, RS, 'ReadableStream').disturbed,
    isReadable: (value) => tryOf(value, RS) !== undefined,
    isErrored: (stream) => of(stream, RS, 'ReadableStream').state === 'errored',
};

module.exports = {
    ReadableStream,
    ReadableStreamDefaultReader,
    ReadableStreamBYOBReader,
    ReadableStreamDefaultController,
    ReadableByteStreamController,
    ReadableStreamBYOBRequest,
    WritableStream,
    WritableStreamDefaultWriter,
    WritableStreamDefaultController,
    TransformStream,
    TransformStreamDefaultController,
    ByteLengthQueuingStrategy,
    CountQueuingStrategy,
    TextEncoderStream,
    TextDecoderStream,
    CompressionStream,
    DecompressionStream,
    internals,
};
