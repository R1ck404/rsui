'use strict';
// Node's `fs`, as far as programs and packages mostly need it. It's only compiled if something asks for it. What's in
// `promises`, and what takes a function to call, happens on another thread, which is what to use for anything that could be
// large or slow. Watching for changes is in `node/misc/fs_watch.js`.

const toBuffer = (bytes) => {
    const { Buffer } = require('internal/buffer');
    return Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
};
const encodingOf = (options) => (typeof options === 'string' ? options : options?.encoding) ?? null;
const isUtf8 = (encoding) => /^utf-?8$/i.test(encoding);

function fromBytes(bytes, encoding) {
    const buffer = toBuffer(bytes);
    return encoding === null || encoding === 'buffer' ? buffer : buffer.toString(encoding);
}

// What's given to be written, as text if it can stay text, as that's what's quickest, or otherwise as bytes.
function dataFor(data, options) {
    if (typeof data === 'string') {
        const encoding = encodingOf(options);
        return encoding === null || isUtf8(encoding) ? data : require('internal/buffer').Buffer.from(data, encoding);
    }
    return ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : String(data);
}

// A path is nearly always text, and text is never a `URL`, so `URL` (and all of `internal/url` with it) is only looked
// at when something else is given.
function pathOf(path) {
    if (typeof path === 'string') {
        return path;
    }
    return path instanceof URL ? require('url').fileURLToPath(path) : String(path);
}

class Stats {
    constructor(found) {
        this.size = found.size;
        this.mtimeMs = found.mtimeMs;
        this.mtime = new Date(found.mtimeMs);
        this.atimeMs = this.ctimeMs = this.birthtimeMs = found.mtimeMs;
        this.atime = this.ctime = this.birthtime = this.mtime;
        this.mode = found.folder ? 0o40666 : 0o100666;
        Object.defineProperty(this, 'kind', { value: found });
    }
    isFile() { return this.kind.file; }
    isDirectory() { return this.kind.folder; }
    isSymbolicLink() { return false; }
    isFIFO() { return false; }
    isSocket() { return false; }
    isBlockDevice() { return false; }
    isCharacterDevice() { return false; }
}

class Dirent {
    constructor(name, parent, stats) {
        this.name = name;
        this.parentPath = this.path = parent;
        Object.defineProperty(this, 'stats', { value: stats });
    }
    isFile() { return this.stats.isFile(); }
    isDirectory() { return this.stats.isDirectory(); }
    isSymbolicLink() { return false; }
}

function readFileSync(path, options) {
    const encoding = encodingOf(options);
    const where = pathOf(path);
    return encoding !== null && isUtf8(encoding) ? __fs.read(where) : fromBytes(__fs.readBytes(where), encoding);
}

function write(path, data, options, append) {
    const given = dataFor(data, options);
    const where = pathOf(path);
    if (typeof given === 'string') {
        __fs.write(where, given, append);
    } else {
        __fs.writeBytes(where, given, append);
    }
}

function writeLater(path, data, options, append) {
    const given = dataFor(data, options);
    const where = pathOf(path);
    return typeof given === 'string' ? __fs.writeLater(where, given, append) : __fs.writeBytesLater(where, given, append);
}

const writeFileSync = (path, data, options) => write(path, data, options, false);
const appendFileSync = (path, data, options) => write(path, data, options, true);
const existsSync = (path) => __fs.exists(pathOf(path));
const unlinkSync = (path) => __fs.remove(pathOf(path), false, false);
const rmSync = (path, options) => __fs.remove(pathOf(path), !!options?.recursive, !!options?.force);
const rmdirSync = (path, options) => __fs.remove(pathOf(path), !!options?.recursive, false);
const renameSync = (from, to) => __fs.rename(pathOf(from), pathOf(to));
const copyFileSync = (from, to) => __fs.copy(pathOf(from), pathOf(to));
/// `{ throwIfNoEntry: false }` has what isn't there be `undefined`, rather than an error, as Node has it.
function statSync(path, options) {
    if (options?.throwIfNoEntry === false && !__fs.exists(pathOf(path))) {
        return undefined;
    }
    return new Stats(__fs.stat(pathOf(path)));
}
const realpathSync = (path) => __fs.realpath(pathOf(path));
const accessSync = (path) => { __fs.stat(pathOf(path)); };

function mkdirSync(path, options) {
    __fs.mkdir(pathOf(path), !!options?.recursive);
    return options?.recursive ? pathOf(path) : undefined;
}

function readdirSync(path, options) {
    const where = pathOf(path);
    const names = __fs.list(where);
    if (!options?.withFileTypes) {
        return names;
    }
    return names.map((name) => new Dirent(name, where, statSync(where + '\\' + name)));
}

// How to open files by number, which is what the layers people write on top of this mostly come down to.
const O_RDWR = 2;
const O_WRONLY = 1;
const O_CREAT = 0x100;
const O_TRUNC = 0x200;
const O_EXCL = 0x400;
const O_APPEND = 8;

// What's rarely used (files by number, file handles, streams and temporary folders) is only compiled the first time one
// of these is called. Each says what it takes as the code it stands for did. See `node/fs_more.js`.
let rarely = null;
const more = () => (rarely ??= require('internal/fs-more')({ __fs, pathOf, encodingOf, promises, closeSync, fstatSync, ftruncateSync, O_RDWR, O_WRONLY, O_CREAT, O_TRUNC, O_EXCL, O_APPEND }));
function openSync(path, flags) {
    return more().openSync(path, flags);
}
function readSync(fd, target, offset = 0, length, position = null) {
    return more().readSync(fd, target, offset, length, position);
}
function writeSync(fd, data, offset, length, position) {
    return more().writeSync(fd, data, offset, length, position);
}
function mkdtempSync(prefix) {
    return more().mkdtempSync(prefix);
}
const fileHandle = (fd) => more().fileHandle(fd);
function createReadStream(path, options) {
    return more().createReadStream(path, options);
}
function createWriteStream(path, options) {
    return more().createWriteStream(path, options);
}

// Watching is compiled the first time something watches.
let watching = null;
const watchers = () => (watching ??= require('internal/misc/fs_watch')({ __fs, pathOf, Stats }));
function watch(filename, options, listener) {
    return watchers().watch(filename, options, listener);
}
function watchFile(filename, options, listener) {
    return watchers().watchFile(filename, options, listener);
}
function unwatchFile(filename, listener) {
    watching?.unwatchFile(filename, listener);
}

const closeSync = (fd) => __fs.close(fd);
const fstatSync = (fd) => new Stats(__fs.fstat(fd));
const ftruncateSync = (fd, length = 0) => __fs.truncate(fd, length);
const fsyncSync = () => {};
const chmodSync = () => {};
const fchmodSync = () => {};
const chownSync = () => {};
const fchownSync = () => {};
const lchownSync = () => {};

const toSeconds = (time) => (time instanceof Date ? time.getTime() : Number(time) * 1000);
const utimesSync = (path, accessed, modified) => __fs.setTime(pathOf(path), toSeconds(modified));
const lutimesSync = utimesSync;
const futimesSync = (fd, accessed, modified) => __fs.setTimeOf(fd, toSeconds(modified));

const promises = {
    async readFile(path, options) {
        const encoding = encodingOf(options);
        const where = pathOf(path);
        if (encoding !== null && isUtf8(encoding)) {
            return __fs.readLater(where);
        }
        return fromBytes(await __fs.readBytesLater(where), encoding);
    },
    writeFile: (path, data, options) => writeLater(path, data, options, false),
    appendFile: (path, data, options) => writeLater(path, data, options, true),
    mkdir: async (path, options) => {
        await __fs.mkdirLater(pathOf(path), !!options?.recursive);
        return options?.recursive ? pathOf(path) : undefined;
    },
    unlink: (path) => __fs.removeLater(pathOf(path), false, false),
    rm: (path, options) => __fs.removeLater(pathOf(path), !!options?.recursive, !!options?.force),
    rmdir: (path, options) => __fs.removeLater(pathOf(path), !!options?.recursive, false),
    rename: (from, to) => __fs.renameLater(pathOf(from), pathOf(to)),
    copyFile: (from, to) => __fs.copyLater(pathOf(from), pathOf(to)),
    realpath: (path) => __fs.realpathLater(pathOf(path)),
    access: async (path) => { await __fs.statLater(pathOf(path)); },
    stat: async (path) => new Stats(await __fs.statLater(pathOf(path))),
    lstat: async (path) => new Stats(await __fs.statLater(pathOf(path))),
    async readdir(path, options) {
        const where = pathOf(path);
        const names = await __fs.listLater(where);
        if (!options?.withFileTypes) {
            return names;
        }
        return Promise.all(names.map(async (name) => new Dirent(name, where, new Stats(await __fs.statLater(where + '\\' + name)))));
    },
    mkdtemp: async (prefix) => mkdtempSync(prefix),
    chmod: async () => {},
    chown: async () => {},
    lchown: async () => {},
    utimes: async (path, accessed, modified) => utimesSync(path, accessed, modified),
    lutimes: async (path, accessed, modified) => utimesSync(path, accessed, modified),
    open: async (path, flags) => fileHandle(openSync(path, flags)),
    watch: (filename, options) => watchers().watchPromised(filename, options),
};

// The way that takes a function to call as its last argument, which is told about it on a later turn.
function withCallback(run) {
    return (...given) => {
        const done = given.pop();
        if (typeof done !== 'function') {
            throw Object.assign(new TypeError('The "cb" argument must be of type function'), { code: 'ERR_INVALID_ARG_TYPE' });
        }
        run(...given).then((value) => done(null, value), (error) => done(error));
    };
}

function exists(path, done) {
    __fs.existsLater(pathOf(path)).then(done);
}

module.exports = {
    readFileSync,
    writeFileSync,
    appendFileSync,
    existsSync,
    mkdirSync,
    mkdtempSync,
    unlinkSync,
    rmSync,
    rmdirSync,
    renameSync,
    copyFileSync,
    readdirSync,
    statSync,
    lstatSync: statSync,
    realpathSync,
    accessSync,
    readFile: withCallback(promises.readFile),
    writeFile: withCallback(promises.writeFile),
    appendFile: withCallback(promises.appendFile),
    mkdir: withCallback(promises.mkdir),
    unlink: withCallback(promises.unlink),
    rm: withCallback(promises.rm),
    rmdir: withCallback(promises.rmdir),
    rename: withCallback(promises.rename),
    copyFile: withCallback(promises.copyFile),
    readdir: withCallback(promises.readdir),
    stat: withCallback(promises.stat),
    lstat: withCallback(promises.lstat),
    realpath: withCallback(promises.realpath),
    access: withCallback(promises.access),
    exists,
    createReadStream,
    createWriteStream,
    watch,
    watchFile,
    unwatchFile,
    mkdtemp: withCallback(promises.mkdtemp),
    openSync,
    closeSync,
    readSync,
    writeSync,
    fstatSync,
    ftruncateSync,
    fsyncSync,
    fdatasyncSync: fsyncSync,
    chmodSync,
    fchmodSync,
    chownSync,
    fchownSync,
    lchownSync,
    utimesSync,
    lutimesSync,
    futimesSync,
    open: withCallback(async (path, flags) => openSync(path, typeof flags === 'function' ? undefined : flags)),
    close: withCallback(async (fd) => closeSync(fd)),
    read: withCallback(async (fd, target, offset, length, position) => readSync(fd, target, offset, length, position)),
    write: withCallback(async (fd, data, offset, length, position) => writeSync(fd, data, offset, length, position)),
    fstat: withCallback(async (fd) => fstatSync(fd)),
    ftruncate: withCallback(async (fd, length) => ftruncateSync(fd, length)),
    fsync: withCallback(async () => {}),
    fdatasync: withCallback(async () => {}),
    chmod: withCallback(async () => {}),
    fchmod: withCallback(async () => {}),
    lchmod: withCallback(async () => {}),
    chown: withCallback(async () => {}),
    fchown: withCallback(async () => {}),
    lchown: withCallback(async () => {}),
    utimes: withCallback(promises.utimes),
    futimes: withCallback(async (fd, accessed, modified) => futimesSync(fd, accessed, modified)),
    lutimes: withCallback(promises.utimes),
    Stats,
    Dirent,
    constants: { F_OK: 0, R_OK: 4, W_OK: 2, X_OK: 1, COPYFILE_EXCL: 1, COPYFILE_FICLONE: 2, COPYFILE_FICLONE_FORCE: 4, O_RDONLY: 0, O_WRONLY, O_RDWR, O_CREAT, O_EXCL, O_TRUNC, O_APPEND, S_IFMT: 0o170000, S_IFREG: 0o100000, S_IFDIR: 0o40000, S_IFLNK: 0o120000 },
    promises,
};
