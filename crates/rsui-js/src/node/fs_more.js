'use strict';
// The parts of `fs` that are rarely used: files by number, file handles, streams and temporary folders. `node/fs.js` only
// compiles this the first time one of them is called, and gives it what of its own these use.

module.exports = ({ __fs, pathOf, encodingOf, promises, closeSync, fstatSync, ftruncateSync, O_RDWR, O_WRONLY, O_CREAT, O_TRUNC, O_EXCL, O_APPEND }) => {
    const FLAGS = { r: 'r', rs: 'r', 'r+': 'r+', 'rs+': 'r+', w: 'w', wx: 'wx', 'w+': 'w+', 'wx+': 'wx+', a: 'a', ax: 'ax', 'a+': 'a+', 'ax+': 'ax+' };

    function flagsFrom(flags) {
        if (typeof flags === 'number') {
            return {
                read: (flags & O_WRONLY) === 0,
                write: (flags & (O_WRONLY | O_RDWR)) !== 0,
                append: (flags & O_APPEND) !== 0,
                create: (flags & O_CREAT) !== 0,
                truncate: (flags & O_TRUNC) !== 0,
                exclusive: (flags & O_EXCL) !== 0,
            };
        }
        const kind = FLAGS[flags ?? 'r'];
        if (kind === undefined) {
            throw Object.assign(new TypeError(`The argument 'flags' is invalid. Received '${flags}'`), { code: 'ERR_INVALID_ARG_VALUE' });
        }
        return {
            read: kind.startsWith('r') || kind.includes('+'),
            write: !kind.startsWith('r') || kind.includes('+'),
            append: kind.startsWith('a'),
            create: !kind.startsWith('r'),
            truncate: kind.startsWith('w'),
            exclusive: kind.includes('x'),
        };
    }

    function openSync(path, flags) {
        const { read, write, append, create, truncate, exclusive } = flagsFrom(flags);
        return __fs.open(pathOf(path), read, write, append, create, truncate, exclusive);
    }

    function readSync(fd, target, offset = 0, length, position = null) {
        if (typeof offset === 'object' && offset !== null) {
            ({ offset = 0, length, position = null } = offset);
        }
        const wanted = length ?? target.byteLength - offset;
        const bytes = __fs.readAt(fd, wanted, position === null || position < 0 ? -1 : Number(position));
        new Uint8Array(target.buffer, target.byteOffset, target.byteLength).set(bytes, offset);
        return bytes.length;
    }

    function writeSync(fd, data, offset, length, position) {
        if (typeof data === 'string') {
            // Here, what comes second is where to write it, and then how it's encoded.
            const encoded = new TextEncoder().encode(data);
            return __fs.writeAt(fd, encoded, typeof offset === 'number' ? offset : -1);
        }
        const view = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
        const from = offset ?? 0;
        return __fs.writeAt(fd, view.subarray(from, from + (length ?? view.length - from)), typeof position === 'number' ? position : -1);
    }

    function mkdtempSync(prefix) {
        const letters = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
        const bytes = crypto.getRandomValues(new Uint8Array(6));
        const where = pathOf(prefix) + Array.from(bytes, (byte) => letters[byte % letters.length]).join('');
        __fs.mkdir(where, false);
        return where;
    }

    function fileHandle(fd) {
        // Once it's closed, its `fd` is -1, and closing it again does nothing, as in Node.
        const handle = {
            fd,
            close: async () => {
                if (handle.fd !== -1) {
                    handle.fd = -1;
                    closeSync(fd);
                }
            },
            async [Symbol.asyncDispose]() {
                await handle.close();
            },
            stat: async () => fstatSync(fd),
            read: async (target, offset, length, position) => ({ bytesRead: readSync(fd, target, offset, length, position), buffer: target }),
            write: async (data, offset, length, position) => ({ bytesWritten: writeSync(fd, data, offset, length, position), buffer: data }),
            sync: async () => {},
            truncate: async (length) => ftruncateSync(fd, length),
        };
        return handle;
    }

    // Streams read or write all of a file at once, on another thread, as nothing here reads part of one.
    function createReadStream(path, options) {
        const { Readable } = require('stream');
        const encoding = encodingOf(options);
        const size = options?.highWaterMark ?? 64 * 1024;
        const stream = new Readable({ encoding, read() {} });
        stream.path = pathOf(path);
        promises.readFile(path).then((bytes) => {
            const from = options?.start ?? 0;
            const to = options?.end === undefined ? bytes.length : options.end + 1;
            for (let at = from; at < to; at += size) {
                stream.push(bytes.subarray(at, Math.min(at + size, to)));
            }
            stream.push(null);
        }, (error) => stream.destroy(error));
        return stream;
    }

    function createWriteStream(path, options) {
        const { Writable } = require('stream');
        const append = typeof options === 'object' && options?.flags?.startsWith('a');
        let first = !append;
        const stream = new Writable({
            write(chunk, encoding, done) {
                const writing = first ? promises.writeFile(path, chunk) : promises.appendFile(path, chunk);
                first = false;
                writing.then(() => done(), done);
            },
            final(done) {
                // A file with nothing written to it still gets made.
                (first ? promises.writeFile(path, '') : Promise.resolve()).then(() => done(), done);
            },
        });
        stream.path = pathOf(path);
        return stream;
    }

    return { openSync, readSync, writeSync, mkdtempSync, fileHandle, createReadStream, createWriteStream };
};
