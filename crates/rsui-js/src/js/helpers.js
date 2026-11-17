'use strict';
// Parts of the standard library that are simplest written in JavaScript. This is only compiled the first time one of
// them is called, so a program that doesn't use them pays nothing. Each is reached through a native function, which
// finds it here by name.
const IteratorPrototype = __iteratorPrototype;
const AsyncIteratorPrototype = Object.getPrototypeOf(Object.getPrototypeOf(async function* () {}.prototype));

function isObject(value) {
    return (typeof value === 'object' && value !== null) || typeof value === 'function';
}

function direct(iterator) {
    if (!isObject(iterator)) throw new TypeError('Iterator helper called on a non-object');
    return { iterator, next: iterator.next };
}

function stepOf(record) {
    const result = record.next.call(record.iterator);
    if (!isObject(result)) throw new TypeError('Iterator result ' + String(result) + ' is not an object');
    return result;
}

function closeQuietly(iterator) {
    try {
        const method = iterator.return;
        if (method !== undefined && method !== null) method.call(iterator);
    } catch {}
}

function close(iterator) {
    const method = iterator.return;
    if (method === undefined || method === null) return;
    const result = method.call(iterator);
    if (!isObject(result)) throw new TypeError('Iterator return result is not an object');
}

const helperStates = new WeakMap();

// DisposableStack and AsyncDisposableStack. Each keeps the same kind of list a block with `using` in it does, which
// `__disposal` (builtins/disposal.rs) adds to and disposes of, so that they dispose in the same way.
const disposal = __disposal;
const ADD = 0, ADD_AWAITED = 1, DISPOSE = 2, STEP = 3, FAILED = 4, ADD_METHOD = 5, NEW_STACK = 6;
const stackStates = new WeakMap();

function stackName(isAsync) {
    return isAsync ? 'AsyncDisposableStack' : 'DisposableStack';
}

function receiverName(value) {
    if (!isObject(value)) return typeof value === 'string' ? JSON.stringify(value) : String(value);
    return typeof value === 'function' ? '#<Function>' : '#<Object>';
}

function stackStateOf(stack, isAsync, method) {
    const state = isObject(stack) ? stackStates.get(stack) : undefined;
    if (state === undefined || state.isAsync !== isAsync) {
        throw new TypeError('Method ' + stackName(isAsync) + '.prototype.' + method + ' called on incompatible receiver ' +
            receiverName(stack));
    }
    return state;
}

function usableStackState(stack, isAsync, method) {
    const state = stackStateOf(stack, isAsync, method);
    if (state.disposed) {
        const name = stackName(isAsync);
        throw new ReferenceError('Cannot call ' + name + '.prototype.' + method + ' on an already-disposed ' + name);
    }
    return state;
}

function stackUse(stack, value, isAsync) {
    const state = usableStackState(stack, isAsync, 'use');
    disposal(isAsync ? ADD_AWAITED : ADD, state.list, value);
    return value;
}

function stackAdopt(stack, value, onDispose, isAsync) {
    const state = usableStackState(stack, isAsync, 'adopt');
    if (typeof onDispose !== 'function') throw new TypeError('onDispose is not a function');
    disposal(ADD_METHOD, state.list, isAsync ? () => onDispose(value) : () => { onDispose(value); }, isAsync);
    return value;
}

function stackDefer(stack, onDispose, isAsync) {
    const state = usableStackState(stack, isAsync, 'defer');
    if (typeof onDispose !== 'function') throw new TypeError('onDispose is not a function');
    disposal(ADD_METHOD, state.list, onDispose, isAsync);
}

function stackMove(stack, isAsync) {
    const state = usableStackState(stack, isAsync, 'move');
    const moved = disposal(NEW_STACK, undefined, isAsync);
    stackStates.set(moved, { list: state.list, disposed: false, isAsync });
    state.list = [];
    state.disposed = true;
    return moved;
}

const HelperPrototype = Object.create(IteratorPrototype);
function defineMethods(target, methods) {
    for (const key of Reflect.ownKeys(methods)) {
        Object.defineProperty(target, key, { value: methods[key], writable: true, enumerable: false, configurable: true });
    }
}
defineMethods(HelperPrototype, {
    next() {
        const state = helperStates.get(this);
        if (!state) throw new TypeError('next method called on incompatible receiver');
        if (state.running) throw new TypeError('Generator is already running');
        if (state.done) return { value: undefined, done: true };
        state.running = true;
        state.started = true;
        try {
            const result = state.step();
            if (result.done) state.done = true;
            return result;
        } catch (error) {
            state.done = true;
            throw error;
        } finally {
            state.running = false;
        }
    },
    return() {
        const state = helperStates.get(this);
        if (!state) throw new TypeError('return method called on incompatible receiver');
        if (state.running) throw new TypeError('Generator is already running');
        if (state.done) return { value: undefined, done: true };
        state.done = true;
        if (state.closeInner) state.closeInner();
        close(state.record.iterator);
        return { value: undefined, done: true };
    },
});
Object.defineProperty(HelperPrototype, Symbol.toStringTag, { value: 'Iterator Helper', configurable: true });

function makeHelper(record, step, closeInner) {
    const helper = Object.create(HelperPrototype);
    helperStates.set(helper, { record, step, closeInner, done: false, running: false, started: false });
    return helper;
}

function needsFunction(iterator, f) {
    if (!isObject(iterator)) throw new TypeError('Iterator helper called on a non-object');
    if (typeof f !== 'function') {
        closeQuietly(iterator);
        throw new TypeError(String(f) + ' is not a function');
    }
}

function callClosing(record, f, value, counter) {
    try {
        return f(value, counter);
    } catch (error) {
        closeQuietly(record.iterator);
        throw error;
    }
}

function toLimit(iterator, limit) {
    if (!isObject(iterator)) throw new TypeError('Iterator helper called on a non-object');
    let number;
    try {
        number = Number(limit);
    } catch (error) {
        closeQuietly(iterator);
        throw error;
    }
    if (number !== number) {
        closeQuietly(iterator);
        throw new RangeError(String(limit) + ' must be positive');
    }
    const whole = Math.trunc(number);
    if (whole < 0) {
        closeQuietly(iterator);
        throw new RangeError(String(limit) + ' must be positive');
    }
    return whole;
}

function getFlattenable(value, rejectStrings) {
    if (!isObject(value)) {
        if (rejectStrings || typeof value !== 'string') throw new TypeError(String(value) + ' is not an object');
    }
    const method = value[Symbol.iterator];
    let iterator = method === undefined || method === null ? value : method.call(value);
    if (!isObject(iterator)) throw new TypeError(String(iterator) + ' is not an object');
    return direct(iterator);
}

const WrapPrototype = Object.create(IteratorPrototype);
const wrapped = new WeakMap();
defineMethods(WrapPrototype, {
    next() {
        const record = wrapped.get(this);
        return record.next.call(record.iterator);
    },
    return() {
        const record = wrapped.get(this);
        const method = record.iterator.return;
        if (method === undefined || method === null) return { value: undefined, done: true };
        return method.call(record.iterator);
    },
});

function escapeRegExpCharacter(character, isFirst) {
    const code = character.charCodeAt(0);
    if (isFirst && /[0-9a-zA-Z]/.test(character)) return '\\x' + code.toString(16).padStart(2, '0');
    if ('^$\\.*+?()[]{}|/'.includes(character)) return '\\' + character;
    if (',-=<>#&!%:;@~\'`"'.includes(character)) return '\\x' + code.toString(16).padStart(2, '0');
    const named = { '\t': '\\t', '\n': '\\n', '\v': '\\v', '\f': '\\f', '\r': '\\r' }[character];
    if (named) return named;
    if (/\s/.test(character) || (code >= 0xD800 && code <= 0xDFFF)) {
        return code < 256 ? '\\x' + code.toString(16).padStart(2, '0') : '\\u' + code.toString(16).padStart(4, '0');
    }
    return character;
}

function setRecord(other) {
    if (!isObject(other)) throw new TypeError('The argument must be an object');
    const rawSize = other.size;
    const size = Number(rawSize);
    if (size !== size) throw new TypeError('The size is not a number');
    const whole = Math.trunc(size);
    if (whole < 0) throw new RangeError('The size is negative');
    const has = other.has;
    if (typeof has !== 'function') throw new TypeError('has is not a function');
    const keys = other.keys;
    if (typeof keys !== 'function') throw new TypeError('keys is not a function');
    return { set: other, size: whole, has, keys };
}

function keysOf(record) {
    const iterator = record.keys.call(record.set);
    if (!isObject(iterator)) throw new TypeError('keys() did not give an object');
    return { iterator, next: iterator.next };
}

const originalSetHas = Set.prototype.has;
const originalSetValues = Set.prototype.values;
function checkSet(set) {
    originalSetHas.call(set, 0);
}
function setItems(set) {
    return [...originalSetValues.call(set)];
}

function canonicalZero(value) {
    return value === 0 ? 0 : value;
}

return {
    // Iterator.prototype
    map(f) {
        needsFunction(this, f);
        const record = direct(this);
        let counter = 0;
        return makeHelper(record, () => {
            const result = stepOf(record);
            if (result.done) return { value: undefined, done: true };
            return { value: callClosing(record, f, result.value, counter++), done: false };
        });
    },
    filter(f) {
        needsFunction(this, f);
        const record = direct(this);
        let counter = 0;
        return makeHelper(record, () => {
            while (true) {
                const result = stepOf(record);
                if (result.done) return { value: undefined, done: true };
                const value = result.value;
                if (callClosing(record, f, value, counter++)) return { value, done: false };
            }
        });
    },
    take(limit) {
        let remaining = toLimit(this, limit);
        const record = direct(this);
        return makeHelper(record, () => {
            if (remaining === 0) {
                close(record.iterator);
                return { value: undefined, done: true };
            }
            if (remaining !== Infinity) remaining--;
            const result = stepOf(record);
            if (result.done) return { value: undefined, done: true };
            return { value: result.value, done: false };
        });
    },
    drop(limit) {
        let remaining = toLimit(this, limit);
        const record = direct(this);
        return makeHelper(record, () => {
            while (remaining > 0) {
                if (remaining !== Infinity) remaining--;
                const skipped = stepOf(record);
                if (skipped.done) return { value: undefined, done: true };
            }
            const result = stepOf(record);
            if (result.done) return { value: undefined, done: true };
            return { value: result.value, done: false };
        });
    },
    flatMap(f) {
        needsFunction(this, f);
        const record = direct(this);
        let counter = 0;
        let inner = null;
        return makeHelper(record, () => {
            while (true) {
                if (inner) {
                    let result;
                    try {
                        result = stepOf(inner);
                    } catch (error) {
                        closeQuietly(record.iterator);
                        throw error;
                    }
                    let done;
                    try {
                        done = result.done;
                    } catch (error) {
                        closeQuietly(record.iterator);
                        throw error;
                    }
                    if (!done) {
                        try {
                            return { value: result.value, done: false };
                        } catch (error) {
                            closeQuietly(record.iterator);
                            throw error;
                        }
                    }
                    inner = null;
                }
                const outer = stepOf(record);
                if (outer.done) return { value: undefined, done: true };
                try {
                    inner = getFlattenable(f(outer.value, counter++), true);
                } catch (error) {
                    closeQuietly(record.iterator);
                    throw error;
                }
            }
        }, () => {
            if (inner) {
                try {
                    close(inner.iterator);
                } catch (error) {
                    closeQuietly(record.iterator);
                    throw error;
                }
            }
        });
    },
    reduce(f, ...rest) {
        needsFunction(this, f);
        const record = direct(this);
        let accumulator;
        let counter = 0;
        if (rest.length === 0) {
            const first = stepOf(record);
            if (first.done) throw new TypeError('Reduce of a done iterator with no initial value');
            accumulator = first.value;
            counter = 1;
        } else {
            accumulator = rest[0];
        }
        while (true) {
            const result = stepOf(record);
            if (result.done) return accumulator;
            const value = result.value;
            try {
                accumulator = f(accumulator, value, counter++);
            } catch (error) {
                closeQuietly(record.iterator);
                throw error;
            }
        }
    },
    toArray() {
        const record = direct(this);
        const out = [];
        while (true) {
            const result = stepOf(record);
            if (result.done) return out;
            out.push(result.value);
        }
    },
    forEach(f) {
        needsFunction(this, f);
        const record = direct(this);
        let counter = 0;
        while (true) {
            const result = stepOf(record);
            if (result.done) return undefined;
            callClosing(record, f, result.value, counter++);
        }
    },
    some(f) {
        needsFunction(this, f);
        const record = direct(this);
        let counter = 0;
        while (true) {
            const result = stepOf(record);
            if (result.done) return false;
            if (callClosing(record, f, result.value, counter++)) {
                close(record.iterator);
                return true;
            }
        }
    },
    every(f) {
        needsFunction(this, f);
        const record = direct(this);
        let counter = 0;
        while (true) {
            const result = stepOf(record);
            if (result.done) return true;
            if (!callClosing(record, f, result.value, counter++)) {
                close(record.iterator);
                return false;
            }
        }
    },
    find(f) {
        needsFunction(this, f);
        const record = direct(this);
        let counter = 0;
        while (true) {
            const result = stepOf(record);
            if (result.done) return undefined;
            const value = result.value;
            if (callClosing(record, f, value, counter++)) {
                close(record.iterator);
                return value;
            }
        }
    },
    // Iterator
    iteratorFrom(value) {
        const record = getFlattenable(value, false);
        if (IteratorPrototype.isPrototypeOf(record.iterator)) return record.iterator;
        const wrapper = Object.create(WrapPrototype);
        wrapped.set(wrapper, record);
        return wrapper;
    },
    // Array
    async arrayFromAsync(items, mapper, thisArg) {
        const C = this;
        const makesOwn = typeof C === 'function' && C !== Array && C.prototype !== undefined;
        if (mapper !== undefined && typeof mapper !== 'function') throw new TypeError('The mapper must be a function');
        const asyncMethod = items[Symbol.asyncIterator];
        const syncMethod = asyncMethod === undefined || asyncMethod === null ? items[Symbol.iterator] : undefined;
        let index = 0;
        if ((asyncMethod !== undefined && asyncMethod !== null) || (syncMethod !== undefined && syncMethod !== null)) {
            const out = makesOwn ? new C() : [];
            for await (const item of items) {
                const value = mapper ? await mapper.call(thisArg, item, index) : item;
                Object.defineProperty(out, index++, { value, writable: true, enumerable: true, configurable: true });
            }
            out.length = index;
            return out;
        }
        const arrayLike = Object(items);
        const length = Math.min(Math.max(Math.trunc(Number(arrayLike.length)) || 0, 0), Number.MAX_SAFE_INTEGER);
        const out = makesOwn ? new C(length) : new Array(length);
        for (; index < length; index++) {
            const item = await arrayLike[index];
            const value = mapper ? await mapper.call(thisArg, item, index) : item;
            Object.defineProperty(out, index, { value, writable: true, enumerable: true, configurable: true });
        }
        out.length = length;
        return out;
    },
    // Set.prototype
    union(other) {
        checkSet(this);
        const record = setRecord(other);
        const iterated = keysOf(record);
        const out = new Set(setItems(this));
        while (true) {
            const result = stepOf(iterated);
            if (result.done) return out;
            out.add(result.value);
        }
    },
    intersection(other) {
        checkSet(this);
        const record = setRecord(other);
        const out = [];
        if (this.size <= record.size) {
            for (const item of setItems(this)) {
                if (originalSetHas.call(this, item) && record.has.call(record.set, item)) out.push(item);
            }
        } else {
            const iterated = keysOf(record);
            while (true) {
                const result = stepOf(iterated);
                if (result.done) break;
                const value = canonicalZero(result.value);
                if (originalSetHas.call(this, value) && !out.includes(value)) out.push(value);
            }
        }
        return new Set(out);
    },
    difference(other) {
        checkSet(this);
        const record = setRecord(other);
        const out = new Set(setItems(this));
        if (this.size <= record.size) {
            for (const item of setItems(this)) {
                if (record.has.call(record.set, item)) out.delete(item);
            }
        } else {
            const iterated = keysOf(record);
            while (true) {
                const result = stepOf(iterated);
                if (result.done) break;
                out.delete(canonicalZero(result.value));
            }
        }
        return out;
    },
    symmetricDifference(other) {
        checkSet(this);
        const record = setRecord(other);
        const iterated = keysOf(record);
        const out = new Set(setItems(this));
        while (true) {
            const result = stepOf(iterated);
            if (result.done) return out;
            const value = canonicalZero(result.value);
            if (originalSetHas.call(this, value)) out.delete(value);
            else out.add(value);
        }
    },
    isSubsetOf(other) {
        checkSet(this);
        const record = setRecord(other);
        if (this.size > record.size) return false;
        for (const item of setItems(this)) {
            if (!record.has.call(record.set, item)) return false;
        }
        return true;
    },
    isSupersetOf(other) {
        checkSet(this);
        const record = setRecord(other);
        if (this.size < record.size) return false;
        const iterated = keysOf(record);
        while (true) {
            const result = stepOf(iterated);
            if (result.done) return true;
            if (!originalSetHas.call(this, result.value)) {
                close(iterated.iterator);
                return false;
            }
        }
    },
    isDisjointFrom(other) {
        checkSet(this);
        const record = setRecord(other);
        if (this.size <= record.size) {
            for (const item of setItems(this)) {
                if (record.has.call(record.set, item)) return false;
            }
            return true;
        }
        const iterated = keysOf(record);
        while (true) {
            const result = stepOf(iterated);
            if (result.done) return true;
            if (originalSetHas.call(this, result.value)) {
                close(iterated.iterator);
                return false;
            }
        }
    },
    // DisposableStack and AsyncDisposableStack
    stackSetUp(isAsync) {
        stackStates.set(this, { list: [], disposed: false, isAsync });
    },
    stackUse(value) {
        return stackUse(this, value, false);
    },
    stackAdopt(value, onDispose) {
        return stackAdopt(this, value, onDispose, false);
    },
    stackDefer(onDispose) {
        stackDefer(this, onDispose, false);
    },
    stackMove() {
        return stackMove(this, false);
    },
    stackDispose() {
        const state = stackStateOf(this, false, 'dispose');
        if (state.disposed) return;
        state.disposed = true;
        disposal(DISPOSE, state.list, 0, undefined);
    },
    stackDisposed() {
        return stackStateOf(this, false, 'disposed').disposed;
    },
    asyncStackUse(value) {
        return stackUse(this, value, true);
    },
    asyncStackAdopt(value, onDisposeAsync) {
        return stackAdopt(this, value, onDisposeAsync, true);
    },
    asyncStackDefer(onDisposeAsync) {
        stackDefer(this, onDisposeAsync, true);
    },
    asyncStackMove() {
        return stackMove(this, true);
    },
    // This waits just as `await using` does, a step at a time.
    async asyncStackDispose() {
        const state = stackStateOf(this, true, 'disposeAsync');
        if (state.disposed) return;
        state.disposed = true;
        const list = state.list;
        let waiting;
        while ((waiting = disposal(STEP, list, 0, undefined)) !== list) {
            try {
                await waiting;
            } catch (error) {
                disposal(FAILED, list, error);
            }
        }
    },
    asyncStackDisposed() {
        return stackStateOf(this, true, 'disposed').disposed;
    },
    // %AsyncIteratorPrototype%[Symbol.asyncDispose]
    async asyncIteratorDispose() {
        const method = this.return;
        if (method === undefined || method === null) return;
        if (typeof method !== 'function') throw new TypeError("An iterator's return isn't a function");
        await method.call(this);
    },
    // Promise
    promiseTry(f, ...args) {
        if (!isObject(this)) throw new TypeError('Promise.try called on a non-object');
        return new this((resolve) => resolve(f(...args)));
    },
    // RegExp
    regExpEscape(text) {
        if (typeof text !== 'string') throw new TypeError('The value to escape must be a string');
        let out = '';
        let isFirst = true;
        for (const character of text) {
            out += character.length === 1 ? escapeRegExpCharacter(character, isFirst) : character;
            isFirst = false;
        }
        return out;
    },
};
