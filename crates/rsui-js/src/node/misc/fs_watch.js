'use strict';
// `fs.watch`, `fs.watchFile` and `fs.promises.watch`, which `node/fs.js` only compiles the first time one is called.
// Windows says what changed, through `builtins/host/watch.rs`, and each watcher asks for what's next once it's told of
// what came before, so a watcher costs nothing while nothing happens.

const EventEmitter = require('events');

module.exports = ({ __fs, pathOf, Stats }) => {
    const ERRNO = { ENOENT: -4058, EPERM: -4048, EIO: -4070 };
    const SAYS = { ENOENT: 'no such file or directory', EPERM: 'operation not permitted', EIO: 'i/o error' };

    const failure = (code, path) =>
        Object.assign(new Error(`${code}: ${SAYS[code] ?? 'unknown error'}, watch${path === null ? '' : ` '${path}'`}`), { errno: ERRNO[code], code, syscall: 'watch', path, filename: path });

    function nameIn(name, encoding) {
        if (encoding === 'buffer' || (encoding && !/^utf-?8$/i.test(encoding))) {
            const bytes = require('internal/buffer').Buffer.from(name);
            return encoding === 'buffer' ? bytes : bytes.toString(encoding);
        }
        return name;
    }

    function abortError(signal) {
        return Object.assign(new Error('The operation was aborted'), { name: 'AbortError', code: 'ABORT_ERR', cause: signal?.reason });
    }

    // Asks for what a watcher sees, for as long as it's open, and gives each to `each`.
    async function follow(id, each) {
        for (;;) {
            const seen = await __fs.watchNext(id);
            if (seen === undefined) {
                return;
            }
            for (const one of seen) {
                if (each(one) === false) {
                    return;
                }
            }
        }
    }

    function optionsOf(options) {
        return typeof options === 'string' ? { encoding: options } : options ?? {};
    }

    class FSWatcher extends EventEmitter {
        #id = null;

        _start(path, options) {
            try {
                this.#id = __fs.watchFolder(path, !!options.recursive, options.persistent !== false);
            } catch (error) {
                throw Object.assign(error, { errno: ERRNO[error.code], filename: path });
            }
            const encoding = options.encoding ?? 'utf8';
            follow(this.#id, (one) => {
                if (this.#id === null) {
                    return false;
                }
                if (one[0] === 'e') {
                    this.#stop();
                    this.emit('error', failure(one.slice(1), path));
                    return false;
                }
                this.emit('change', one[0] === 'c' ? 'change' : 'rename', one.length === 1 ? null : nameIn(one.slice(1), encoding));
            });
        }

        #stop() {
            if (this.#id !== null) {
                __fs.watchStop(this.#id);
                this.#id = null;
                return true;
            }
            return false;
        }

        close() {
            if (this.#stop()) {
                process.nextTick(() => this.emit('close'));
            }
        }

        ref() {
            if (this.#id !== null) {
                __fs.watchHold(this.#id, true);
            }
            return this;
        }

        unref() {
            if (this.#id !== null) {
                __fs.watchHold(this.#id, false);
            }
            return this;
        }
    }

    function watch(filename, options, listener) {
        if (typeof options === 'function') {
            listener = options;
            options = {};
        }
        options = optionsOf(options);
        const path = pathOf(filename);
        const watcher = new FSWatcher();
        watcher._start(path, options);
        if (listener) {
            watcher.on('change', listener);
        }
        const signal = options.signal;
        if (signal) {
            if (signal.aborted) {
                process.nextTick(() => watcher.close());
            } else {
                const stop = () => watcher.close();
                signal.addEventListener('abort', stop, { once: true });
                watcher.once('close', () => signal.removeEventListener('abort', stop));
            }
        }
        return watcher;
    }

    // `fs.promises.watch`, as something to go through with `for await`, which ends with an `AbortError` when its
    // signal says so.
    function watchPromised(filename, options) {
        options = optionsOf(options);
        const { signal, maxQueue = 2048, overflow = 'ignore' } = options;
        const queue = [];
        let wake = null;
        let failed = null;
        let watcher = null;
        const notify = () => {
            wake?.();
            wake = null;
        };
        const end = () => {
            watcher?.close();
            watcher = null;
        };
        return {
            async next() {
                if (watcher === null && failed === null && queue.length === 0) {
                    if (signal?.aborted) {
                        throw abortError(signal);
                    }
                    watcher = watch(filename, { ...options, signal: undefined });
                    watcher.on('change', (eventType, name) => {
                        if (queue.length < maxQueue) {
                            queue.push({ eventType, filename: name });
                        } else if (overflow === 'throw') {
                            failed = Object.assign(new Error(`fs.watch maxQueue exceeded`), { code: 'ERR_FS_WATCH_QUEUE_OVERFLOW' });
                            end();
                        }
                        notify();
                    });
                    watcher.on('error', (error) => {
                        failed = error;
                        notify();
                    });
                    signal?.addEventListener('abort', () => {
                        failed = abortError(signal);
                        end();
                        notify();
                    }, { once: true });
                }
                for (;;) {
                    if (queue.length > 0) {
                        return { value: queue.shift(), done: false };
                    }
                    if (failed !== null) {
                        throw failed;
                    }
                    if (watcher === null) {
                        return { value: undefined, done: true };
                    }
                    await new Promise((resolve) => {
                        wake = resolve;
                    });
                }
            },
            async return() {
                end();
                return { value: undefined, done: true };
            },
            [Symbol.asyncIterator]() {
                return this;
            },
        };
    }

    // `fs.watchFile` looks at a file every so often, on the watching thread, and says when it's different.
    const statOf = (kind, size, accessed, modified, born) => {
        const stats = new Stats({ file: kind === 1, folder: kind === 2, size, mtimeMs: modified });
        stats.atimeMs = accessed;
        stats.atime = new Date(accessed);
        stats.birthtimeMs = born;
        stats.birthtime = new Date(born);
        if (kind === 0) {
            stats.mode = 0;
        }
        return Object.assign(stats, { dev: 0, nlink: kind === 0 ? 0 : 1, uid: 0, gid: 0, rdev: 0, blksize: 4096, ino: 0, blocks: Math.ceil(size / 4096) * 8 });
    };

    class StatWatcher extends EventEmitter {
        _start(path, options) {
            this._id = __fs.watchPoll(path, options.interval ?? 5007, options.persistent !== false);
            follow(this._id, (one) => {
                if (this._id === null) {
                    return false;
                }
                const n = one.slice(1).split(',').map(Number);
                this.emit('change', statOf(...n.slice(0, 5)), statOf(...n.slice(5)));
            });
        }

        stop() {
            if (this._id !== null) {
                __fs.watchStop(this._id);
                this._id = null;
                process.nextTick(() => this.emit('stop'));
            }
        }

        ref() {
            if (this._id !== null) {
                __fs.watchHold(this._id, true);
            }
            return this;
        }

        unref() {
            if (this._id !== null) {
                __fs.watchHold(this._id, false);
            }
            return this;
        }
    }

    const watching = new Map();

    function watchFile(filename, options, listener) {
        if (typeof options === 'function') {
            listener = options;
            options = {};
        }
        if (typeof listener !== 'function') {
            throw Object.assign(new TypeError('The "listener" argument must be of type function'), { code: 'ERR_INVALID_ARG_TYPE' });
        }
        const path = pathOf(filename);
        let watcher = watching.get(path);
        if (watcher === undefined) {
            watcher = new StatWatcher();
            watcher._start(path, options ?? {});
            watching.set(path, watcher);
        }
        watcher.on('change', listener);
        return watcher;
    }

    function unwatchFile(filename, listener) {
        const path = pathOf(filename);
        const watcher = watching.get(path);
        if (watcher === undefined) {
            return;
        }
        if (typeof listener === 'function') {
            watcher.removeListener('change', listener);
        } else {
            watcher.removeAllListeners('change');
        }
        if (watcher.listenerCount('change') === 0) {
            watcher.stop();
            watching.delete(path);
        }
    }

    return { watch, watchFile, unwatchFile, watchPromised };
};
