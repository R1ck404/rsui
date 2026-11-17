'use strict';
// Node's `events`, for packages that expect it. It's only compiled if something asks for it. It's a plain function rather than
// a class, as a lot of what's written for Node calls it as one, with `EventEmitter.call(this)`.
function EventEmitter(options) {
    if (!(this instanceof EventEmitter)) {
        return new EventEmitter(options);
    }
    this._events = Object.create(null);
    this._maxListeners = undefined;
}

const eventsOf = (emitter) => {
    if (!emitter._events) {
        emitter._events = Object.create(null);
    }
    return emitter._events;
};

function add(emitter, name, listener, first) {
    if (typeof listener !== 'function') {
        throw Object.assign(new TypeError('The "listener" argument must be of type function.'), { code: 'ERR_INVALID_ARG_TYPE' });
    }
    const events = eventsOf(emitter);
    if (events.newListener !== undefined) {
        emitter.emit('newListener', name, listener.listener ?? listener);
    }
    const list = events[name];
    if (list === undefined) {
        events[name] = [listener];
    } else if (first) {
        list.unshift(listener);
    } else {
        list.push(listener);
    }
    return emitter;
}

function onceWrapper(emitter, name, listener) {
    const wrapper = function (...args) {
        emitter.removeListener(name, wrapper);
        return listener.apply(emitter, args);
    };
    wrapper.listener = listener;
    return wrapper;
}

Object.assign(EventEmitter.prototype, {
    on(name, listener) {
        return add(this, name, listener, false);
    },
    addListener(name, listener) {
        return add(this, name, listener, false);
    },
    prependListener(name, listener) {
        return add(this, name, listener, true);
    },
    once(name, listener) {
        if (typeof listener !== 'function') {
            throw Object.assign(new TypeError('The "listener" argument must be of type function.'), { code: 'ERR_INVALID_ARG_TYPE' });
        }
        // As Node does, by way of `on`, which is what a stream looks at to start flowing.
        return this.on(name, onceWrapper(this, name, listener));
    },
    prependOnceListener(name, listener) {
        return this.prependListener(name, onceWrapper(this, name, listener));
    },
    off(name, listener) {
        return this.removeListener(name, listener);
    },
    removeListener(name, listener) {
        const events = eventsOf(this);
        const list = events[name];
        if (list === undefined) {
            return this;
        }
        // The last one added goes first, as Node has it.
        for (let at = list.length - 1; at >= 0; at -= 1) {
            if (list[at] === listener || list[at].listener === listener) {
                list.splice(at, 1);
                if (list.length === 0) {
                    delete events[name];
                }
                if (events.removeListener !== undefined) {
                    this.emit('removeListener', name, listener);
                }
                break;
            }
        }
        return this;
    },
    removeAllListeners(name) {
        if (name === undefined) {
            this._events = Object.create(null);
        } else {
            delete eventsOf(this)[name];
        }
        return this;
    },
    emit(name, ...args) {
        const list = eventsOf(this)[name];
        if (list === undefined || list.length === 0) {
            if (name === 'error') {
                if (args[0] instanceof Error) {
                    throw args[0];
                }
                throw Object.assign(new Error('Unhandled error. (' + String(args[0]) + ')'), { code: 'ERR_UNHANDLED_ERROR', context: args[0] });
            }
            return false;
        }
        for (const listener of list.slice()) {
            listener.apply(this, args);
        }
        return true;
    },
    listeners(name) {
        return (eventsOf(this)[name] ?? []).map((found) => found.listener ?? found);
    },
    rawListeners(name) {
        return [...(eventsOf(this)[name] ?? [])];
    },
    listenerCount(name) {
        return eventsOf(this)[name]?.length ?? 0;
    },
    eventNames() {
        return Reflect.ownKeys(eventsOf(this));
    },
    setMaxListeners(count) {
        this._maxListeners = count;
        return this;
    },
    getMaxListeners() {
        return this._maxListeners ?? EventEmitter.defaultMaxListeners;
    },
});

EventEmitter.EventEmitter = EventEmitter;
EventEmitter.defaultMaxListeners = 10;
EventEmitter.errorMonitor = Symbol('events.errorMonitor');
EventEmitter.captureRejectionSymbol = Symbol.for('nodejs.rejection');
EventEmitter.listenerCount = (emitter, name) => emitter.listenerCount(name);
EventEmitter.setMaxListeners = () => {};
EventEmitter.getEventListeners = (emitter, name) => emitter.listeners(name);
EventEmitter.once = (emitter, name) => new Promise((resolve, reject) => {
    const onError = (error) => {
        emitter.off(name, onValue);
        reject(error);
    };
    const onValue = (...args) => {
        emitter.off('error', onError);
        resolve(args);
    };
    emitter.once(name, onValue);
    if (name !== 'error') {
        emitter.once('error', onError);
    }
});
module.exports = EventEmitter;
