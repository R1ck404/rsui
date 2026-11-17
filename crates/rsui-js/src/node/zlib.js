'use strict';
// Node's `zlib`: DEFLATE (with zlib's header, gzip's, or neither), Brotli and Zstandard. The work is done in Rust
// (`builtins/host/zlib.rs`), all at once for what's `Sync`, on another thread for what takes a function to call, and a
// piece at a time for the streams, which give out what they can as soon as there's something.

const { Transform } = require('stream');

const BrotliConstants = {
    BROTLI_OPERATION_PROCESS: 0, BROTLI_OPERATION_FLUSH: 1, BROTLI_OPERATION_FINISH: 2, BROTLI_OPERATION_EMIT_METADATA: 3,
    BROTLI_PARAM_MODE: 0, BROTLI_MODE_GENERIC: 0, BROTLI_MODE_TEXT: 1, BROTLI_MODE_FONT: 2, BROTLI_DEFAULT_MODE: 0, BROTLI_PARAM_QUALITY: 1, BROTLI_MIN_QUALITY: 0,
    BROTLI_MAX_QUALITY: 11, BROTLI_DEFAULT_QUALITY: 11, BROTLI_PARAM_LGWIN: 2, BROTLI_MIN_WINDOW_BITS: 10, BROTLI_MAX_WINDOW_BITS: 24, BROTLI_LARGE_MAX_WINDOW_BITS: 30,
    BROTLI_DEFAULT_WINDOW: 22, BROTLI_PARAM_LGBLOCK: 3, BROTLI_MIN_INPUT_BLOCK_BITS: 16, BROTLI_MAX_INPUT_BLOCK_BITS: 24, BROTLI_PARAM_DISABLE_LITERAL_CONTEXT_MODELING: 4,
    BROTLI_PARAM_SIZE_HINT: 5, BROTLI_PARAM_LARGE_WINDOW: 6, BROTLI_PARAM_NPOSTFIX: 7, BROTLI_PARAM_NDIRECT: 8, BROTLI_DECODER_RESULT_ERROR: 0, BROTLI_DECODER_RESULT_SUCCESS: 1,
    BROTLI_DECODER_RESULT_NEEDS_MORE_INPUT: 2, BROTLI_DECODER_RESULT_NEEDS_MORE_OUTPUT: 3, BROTLI_DECODER_PARAM_DISABLE_RING_BUFFER_REALLOCATION: 0, BROTLI_DECODER_PARAM_LARGE_WINDOW: 1,
    BROTLI_DECODER_NO_ERROR: 0, BROTLI_DECODER_SUCCESS: 1, BROTLI_DECODER_NEEDS_MORE_INPUT: 2, BROTLI_DECODER_NEEDS_MORE_OUTPUT: 3,
};
// Brotli's errors, which are numbered from -1 down, with gaps.
for (const [at, name] of ['EXUBERANT_NIBBLE', 'RESERVED', 'EXUBERANT_META_NIBBLE', 'SIMPLE_HUFFMAN_ALPHABET', 'SIMPLE_HUFFMAN_SAME', 'CL_SPACE', 'HUFFMAN_SPACE', 'CONTEXT_MAP_REPEAT', 'BLOCK_LENGTH_1', 'BLOCK_LENGTH_2', 'TRANSFORM', 'DICTIONARY', 'WINDOW_BITS', 'PADDING_1', 'PADDING_2', 'DISTANCE'].entries()) {
    BrotliConstants['BROTLI_DECODER_ERROR_FORMAT_' + name] = -1 - at;
}
Object.assign(BrotliConstants, {
    BROTLI_DECODER_ERROR_DICTIONARY_NOT_SET: -19, BROTLI_DECODER_ERROR_INVALID_ARGUMENTS: -20, BROTLI_DECODER_ERROR_ALLOC_CONTEXT_MODES: -21, BROTLI_DECODER_ERROR_ALLOC_TREE_GROUPS: -22,
    BROTLI_DECODER_ERROR_ALLOC_CONTEXT_MAP: -25, BROTLI_DECODER_ERROR_ALLOC_RING_BUFFER_1: -26, BROTLI_DECODER_ERROR_ALLOC_RING_BUFFER_2: -27, BROTLI_DECODER_ERROR_ALLOC_BLOCK_TYPE_TREES: -30,
    BROTLI_DECODER_ERROR_UNREACHABLE: -31,
});

const ZstdConstants = { ZSTD_e_continue: 0, ZSTD_e_flush: 1, ZSTD_e_end: 2 };
for (const [at, name] of ['fast', 'dfast', 'greedy', 'lazy', 'lazy2', 'btlazy2', 'btopt', 'btultra', 'btultra2'].entries()) {
    ZstdConstants['ZSTD_' + name] = at + 1;
}
// zstd's parameters, and its errors, each run numbered from where it starts, with a dot for a number that's skipped.
for (const [prefix, from, names] of [
    ['ZSTD_c_', 100, 'compressionLevel windowLog hashLog chainLog searchLog minMatch targetLength strategy'],
    ['ZSTD_c_', 160, 'enableLongDistanceMatching ldmHashLog ldmMinMatch ldmBucketSizeLog ldmHashRateLog'],
    ['ZSTD_c_', 200, 'contentSizeFlag checksumFlag dictIDFlag'],
    ['ZSTD_c_', 400, 'nbWorkers jobSize overlapLog'],
    ['ZSTD_d_', 100, 'windowLogMax'],
    ['ZSTD_', 3, 'CLEVEL_DEFAULT'],
    ['ZSTD_error_', 0, 'no_error GENERIC'],
    ['ZSTD_error_', 10, 'prefix_unknown . version_unsupported . frameParameter_unsupported . frameParameter_windowTooLarge . . . corruption_detected . checksum_wrong . literals_headerWrong'],
    ['ZSTD_error_', 30, 'dictionary_corrupted . dictionary_wrong . dictionaryCreation_failed'],
    ['ZSTD_error_', 40, 'parameter_unsupported parameter_combination_unsupported parameter_outOfBound . tableLog_tooLarge . maxSymbolValue_tooLarge . maxSymbolValue_tooSmall . stabilityCondition_notRespected'],
    ['ZSTD_error_', 60, 'stage_wrong . init_missing . memory_allocation . workSpace_tooSmall'],
    ['ZSTD_error_', 70, 'dstSize_tooSmall . srcSize_wrong . dstBuffer_null'],
    ['ZSTD_error_', 80, 'noForwardProgress_destFull . noForwardProgress_inputEmpty'],
]) {
    for (const [at, name] of names.split(' ').entries()) {
        if (name !== '.') {
            ZstdConstants[prefix + name] = from + at;
        }
    }
}

const constants = Object.freeze({
    Z_NO_FLUSH: 0, Z_PARTIAL_FLUSH: 1, Z_SYNC_FLUSH: 2, Z_FULL_FLUSH: 3, Z_FINISH: 4, Z_BLOCK: 5, Z_OK: 0, Z_STREAM_END: 1, Z_NEED_DICT: 2, Z_ERRNO: -1, Z_STREAM_ERROR: -2,
    Z_DATA_ERROR: -3, Z_MEM_ERROR: -4, Z_BUF_ERROR: -5, Z_VERSION_ERROR: -6, Z_NO_COMPRESSION: 0, Z_BEST_SPEED: 1, Z_BEST_COMPRESSION: 9, Z_DEFAULT_COMPRESSION: -1, Z_FILTERED: 1,
    Z_HUFFMAN_ONLY: 2, Z_RLE: 3, Z_FIXED: 4, Z_DEFAULT_STRATEGY: 0, ZLIB_VERNUM: 4880, DEFLATE: 1, INFLATE: 2, GZIP: 3, GUNZIP: 4, DEFLATERAW: 5, INFLATERAW: 6, UNZIP: 7,
    BROTLI_DECODE: 8, BROTLI_ENCODE: 9, ZSTD_DECOMPRESS: 11, ZSTD_COMPRESS: 10, Z_MIN_WINDOWBITS: 8, Z_MAX_WINDOWBITS: 15, Z_DEFAULT_WINDOWBITS: 15, Z_MIN_CHUNK: 64, Z_MAX_CHUNK: Infinity, Z_DEFAULT_CHUNK: 16384, Z_MIN_MEMLEVEL: 1, Z_MAX_MEMLEVEL: 9,
    Z_DEFAULT_MEMLEVEL: 8, Z_MIN_LEVEL: -1, Z_MAX_LEVEL: 9, Z_DEFAULT_LEVEL: -1,
    ...BrotliConstants,
    ...ZstdConstants,
});

const codes = { Z_OK: 0, Z_STREAM_END: 1, Z_NEED_DICT: 2, Z_ERRNO: -1, Z_STREAM_ERROR: -2, Z_DATA_ERROR: -3, Z_MEM_ERROR: -4, Z_BUF_ERROR: -5, Z_VERSION_ERROR: -6 };
for (const [name, number] of Object.entries(codes)) {
    codes[number] = name;
}
Object.freeze(codes);

// Which of these is done, by its number in Rust.
const KINDS = { Deflate: 0, Inflate: 1, Gzip: 2, Gunzip: 3, DeflateRaw: 4, InflateRaw: 5, Unzip: 6, BrotliCompress: 7, BrotliDecompress: 8, ZstdCompress: 9, ZstdDecompress: 10 };

const bufferOf = (bytes) => require('internal/buffer').Buffer.from(bytes);

function inputOf(data) {
    if (typeof data === 'string') {
        return require('internal/buffer').Buffer.from(data);
    }
    if (data instanceof ArrayBuffer) {
        return new Uint8Array(data);
    }
    if (ArrayBuffer.isView(data)) {
        return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    }
    throw Object.assign(new TypeError(`The "buffer" argument must be of type string or an instance of Buffer, TypedArray, DataView, or ArrayBuffer. Received ${data === null ? 'null' : `type ${typeof data} (${String(data)})`}`), { code: 'ERR_INVALID_ARG_TYPE' });
}

// The error for `code:message` from Rust, as zlib's binding in Node makes it.
function failure(text) {
    const split = text.indexOf(':');
    const code = text.slice(0, split);
    const errno = code in codes ? codes[code] : code.startsWith('ERR__') ? constants['BROTLI_DECODER' + code.slice(4)] : constants[code];
    return Object.assign(new Error(text.slice(split + 1)), { errno, code });
}

function checked(options, name, min, max, otherwise) {
    const value = options?.[name];
    if (value === undefined || value === null) {
        return otherwise;
    }
    if (typeof value !== 'number') {
        throw Object.assign(new TypeError(`The "options.${name}" property must be of type number. Received type ${typeof value} (${String(value)})`), { code: 'ERR_INVALID_ARG_TYPE' });
    }
    if (!(value >= min && value <= max)) {
        throw Object.assign(new RangeError(`The value of "options.${name}" is out of range. It must be >= ${min} and <= ${max}. Received ${value}`), { code: 'ERR_OUT_OF_RANGE' });
    }
    return value;
}

// What each zstd parameter can be, past -1, which leaves it be: 0, for what the level says, or from the least to the
// most. A number up to the last one here that isn't here is one zstd doesn't have.
const ANY = [-(2 ** 31), 2 ** 31];
const ZSTD_BOUNDS = [
    { 10: [0, 1], 100: ANY, 101: [10, 31], 102: [6, 30], 103: [6, 30], 104: [0, 30], 105: [3, 7], 106: [0, 131072], 107: [0, 9], 130: [-(2 ** 31), 131072], 160: [0, 2], 161: [6, 30], 162: [4, 4096], 163: [0, 8], 164: [0, 25], 200: ANY, 201: ANY, 202: ANY, 400: ANY, 401: ANY, 402: ANY },
    { 100: [10, 31] },
];

const received = (value) => (value === null || value === undefined ? String(value) : `type ${typeof value} (${typeof value === 'string' ? `'${value}'` : String(value)})`);

// zstd's parameters, as keys each followed by its value, checked as Node checks them.
function zstdParams(kind, options) {
    const bounds = ZSTD_BOUNDS[kind - KINDS.ZstdCompress];
    const most = kind === KINDS.ZstdCompress ? 402 : 100;
    const pairs = [];
    const seen = new Set();
    for (const key of Object.keys(options?.params ?? {})) {
        const number = +key;
        if (Number.isNaN(number) || number < 0 || number > most || seen.has(number | 0)) {
            throw Object.assign(new RangeError(`${key} is not a valid zstd parameter`), { code: 'ERR_ZSTD_INVALID_PARAM' });
        }
        seen.add(number | 0);
        const value = options.params[key];
        if (typeof value !== 'number' && typeof value !== 'boolean') {
            throw Object.assign(new TypeError(`The "options.params[key]" property must be of type number. Received ${received(value)}`), { code: 'ERR_INVALID_ARG_TYPE' });
        }
        // Node hands each over as an unsigned 32-bit number, and all ones means it wasn't given.
        if (value >>> 0 !== 0xffffffff) {
            pairs.push(number | 0, value | 0);
        }
    }
    for (let at = 0; at < pairs.length; at += 2) {
        const range = bounds[pairs[at]];
        if (range === undefined || (pairs[at + 1] !== 0 && !(pairs[at + 1] >= range[0] && pairs[at + 1] <= range[1]))) {
            throw Object.assign(new Error('Setting parameter failed'), { code: 'ERR_ZLIB_INITIALIZATION_FAILED' });
        }
    }
    return new Int32Array(pairs);
}

// What Rust is told to make: `[kind, level, windowBits, memLevel, strategy, dictionary, sizeHint, params]`.
function recipeOf(kind, options) {
    if (kind >= KINDS.ZstdCompress) {
        // Only a view of bytes is taken as a dictionary, as in Node, and the size the input's said to be is one more.
        const dictionary = ArrayBuffer.isView(options?.dictionary) ? inputOf(options.dictionary) : undefined;
        const pledged = options?.pledgedSrcSize;
        return [kind, 3, 0, 0, 0, dictionary, typeof pledged === 'number' ? pledged + 1 : 0, zstdParams(kind, options)];
    }
    if (kind >= KINDS.BrotliCompress) {
        let quality = 11;
        let window = 22;
        let mode = 0;
        let hint = 0;
        for (const [key, value] of Object.entries(options?.params ?? {})) {
            const number = Number(key);
            if (!(number >= 0 && number <= 8) || !Number.isInteger(number)) {
                throw Object.assign(new RangeError(`${key} is not a valid Brotli parameter`), { code: 'ERR_BROTLI_INVALID_PARAM' });
            }
            if (number === constants.BROTLI_PARAM_QUALITY) {
                quality = Math.min(11, Math.max(0, value | 0));
            } else if (number === constants.BROTLI_PARAM_LGWIN) {
                window = Math.min(24, Math.max(10, value | 0));
            } else if (number === constants.BROTLI_PARAM_MODE) {
                mode = value | 0;
            } else if (number === constants.BROTLI_PARAM_SIZE_HINT) {
                hint = value >>> 0;
            }
        }
        return [kind, quality, window, mode, 0, undefined, hint];
    }
    const inflating = kind === KINDS.Inflate || kind === KINDS.Gunzip || kind === KINDS.InflateRaw || kind === KINDS.Unzip;
    let windowBits = options?.windowBits;
    if (inflating && windowBits === 0) {
        windowBits = 0;
    } else {
        windowBits = checked(options, 'windowBits', inflating ? 8 : 9, 15, 15);
    }
    const level = checked(options, 'level', -1, 9, -1);
    const memLevel = checked(options, 'memLevel', 1, 9, 8);
    const strategy = checked(options, 'strategy', 0, 4, 0);
    const dictionary = options?.dictionary === undefined ? undefined : inputOf(options.dictionary);
    return [kind, level, windowBits, memLevel, strategy, dictionary, 0];
}

// The flushes each kind has, from none to the most, then its usual flush, end and full flush.
const flushesOf = (kind) => (kind >= KINDS.ZstdCompress ? [0, 2, 0, 2, 1] : kind >= KINDS.BrotliCompress ? [0, 3, 0, 2, 1] : [0, 5, 0, 4, 3]);

// What every kind takes, checked as Node checks it once what's only for that kind has been.
function optionsOf(kind, options) {
    const [least, most, flush, finish, full] = flushesOf(kind);
    let chunkSize = options?.chunkSize;
    if (chunkSize === undefined || Number.isNaN(chunkSize)) {
        chunkSize = 16384;
    } else if (typeof chunkSize !== 'number') {
        throw Object.assign(new TypeError(`The "options.chunkSize" property must be of type number. Received ${received(chunkSize)}`), { code: 'ERR_INVALID_ARG_TYPE' });
    } else if (!(chunkSize >= 64) || chunkSize === Infinity) {
        throw Object.assign(new RangeError(`The value of "options.chunkSize" is out of range. It must be ${chunkSize === Infinity ? 'a finite number' : '>= 64'}. Received ${chunkSize}`), { code: 'ERR_OUT_OF_RANGE' });
    }
    return {
        chunkSize,
        flush: checked(options, 'flush', least, most, flush),
        finish: checked(options, 'finishFlush', least, most, finish),
        maxOutputLength: checked(options, 'maxOutputLength', 1, 2 ** 53 - 1, 2 ** 53 - 1),
        full: options?.fullFlush ?? full,
    };
}

function tooLarge(size) {
    return Object.assign(new RangeError(`Cannot create a Buffer larger than ${size} bytes`), { code: 'ERR_BUFFER_TOO_LARGE' });
}

// Each kind of flush is written as an empty buffer of its own, so the stream knows it's one when it comes to it.
const FLUSHING = new Map();
const flushBuffer = (kind) => {
    let found = FLUSHING.get(kind);
    if (found === undefined) {
        // A `Buffer`, which a stream passes on as it is, where other bytes become a new one.
        found = require('internal/buffer').Buffer.alloc(0);
        FLUSHING.set(kind, found);
    }
    return found;
};
const isFlush = (chunk) => chunk.length === 0 && [...FLUSHING.values()].includes(chunk);
const flushKindOf = (chunk) => [...FLUSHING.entries()].find(([, buffer]) => buffer === chunk)[0];

// What's let go of when a stream is gone without being closed.
const closer = new FinalizationRegistry((id) => __fs.zlibClose(id));

class ZlibBase extends Transform {
    constructor(options, kind) {
        const recipe = recipeOf(kind, options);
        const checks = optionsOf(kind, options);
        super({ highWaterMark: options?.highWaterMark, readableHighWaterMark: options?.readableHighWaterMark, writableHighWaterMark: options?.writableHighWaterMark, encoding: options?.encoding });
        const id = __fs.zlibOpen(...recipe);
        if (typeof id === 'string') {
            throw failure(id);
        }
        this.bytesWritten = 0;
        this._handle = { id, kind };
        closer.register(this, this._handle.id, this._handle);
        this._chunkSize = checks.chunkSize;
        this._defaultFlushFlag = checks.flush;
        this._finishFlushFlag = checks.finish;
        this._defaultFullFlushFlag = checks.full;
        this._info = !!options?.info;
        this._maxOutputLength = checks.maxOutputLength;
        this._rejectGarbageAfterEnd = !!options?.rejectGarbageAfterEnd;
        this._outputSize = 0;
    }

    // Gives what Rust makes of `chunk`, in pieces no longer than `chunkSize`.
    _run(chunk, flag) {
        if (this._handle === null) {
            throw Object.assign(new Error('zlib binding closed'), { code: 'ERR_ZLIB_CLOSED' });
        }
        const out = __fs.zlibPush(this._handle.id, chunk, flag);
        if (typeof out === 'string') {
            throw failure(out);
        }
        this.bytesWritten += chunk.length;
        this._outputSize += out.byteLength;
        if (this._outputSize > this._maxOutputLength) {
            throw tooLarge(this._maxOutputLength);
        }
        const bytes = bufferOf(out);
        for (let at = 0; at < bytes.length; at += this._chunkSize) {
            this.push(at === 0 && bytes.length <= this._chunkSize ? bytes : bytes.subarray(at, at + this._chunkSize));
        }
        // Zstandard ends when more comes after a frame in the same piece, which can be taken as wrong.
        if (this._rejectGarbageAfterEnd && this._handle.kind === KINDS.ZstdDecompress && __fs.zlibEnded(this._handle.id)) {
            throw Object.assign(new TypeError('Trailing junk found after the end of the compressed stream'), { code: 'ERR_TRAILING_JUNK_AFTER_STREAM_END' });
        }
    }

    get _closed() {
        return this._handle === null;
    }

    _transform(chunk, encoding, done) {
        const flag = isFlush(chunk) ? flushKindOf(chunk) : this._defaultFlushFlag;
        try {
            this._run(chunk, flag);
        } catch (error) {
            done(error);
            return;
        }
        done();
    }

    _flush(done) {
        try {
            this._run(new Uint8Array(0), this._finishFlushFlag);
        } catch (error) {
            this.destroy(error);
            return;
        }
        done();
    }

    flush(kind, callback) {
        if (typeof kind === 'function' || kind === undefined) {
            callback = typeof kind === 'function' ? kind : callback;
            kind = this._defaultFullFlushFlag;
        }
        if (this.writableFinished) {
            if (callback) {
                process.nextTick(callback);
            }
        } else if (this.writableEnded) {
            if (callback) {
                this.once('end', callback);
            }
        } else {
            this.write(flushBuffer(kind), '', callback);
        }
    }

    reset() {
        if (this._handle !== null) {
            __fs.zlibReset(this._handle.id);
        }
    }

    close(callback) {
        if (callback) {
            this.once('close', callback);
        }
        this.destroy();
    }

    _destroy(error, done) {
        if (this._handle !== null) {
            closer.unregister(this._handle);
            __fs.zlibClose(this._handle.id);
            this._handle = null;
        }
        done(error);
    }
}

class Zlib extends ZlibBase {
    constructor(options, kind) {
        super(options, kind);
        this._level = options?.level ?? constants.Z_DEFAULT_COMPRESSION;
        this._strategy = options?.strategy ?? constants.Z_DEFAULT_STRATEGY;
    }

    params(level, strategy, callback) {
        checked({ level }, 'level', -1, 9, -1);
        checked({ strategy }, 'strategy', 0, 4, 0);
        if (this._level === level && this._strategy === strategy) {
            if (callback) {
                process.nextTick(callback);
            }
            return;
        }
        this.flush(constants.Z_SYNC_FLUSH, () => {
            if (this._handle !== null) {
                __fs.zlibParams(this._handle.id, level, strategy);
                this._level = level;
                this._strategy = strategy;
            }
            callback?.();
        });
    }
}

class Brotli extends ZlibBase {}

class Zstd extends ZlibBase {}

const exported = { constants, codes, crc32: (data, value = 0) => __fs.crc32(inputOf(data), value) };

for (const [name, method] of [
    ['Deflate', 'deflate'],
    ['Inflate', 'inflate'],
    ['Gzip', 'gzip'],
    ['Gunzip', 'gunzip'],
    ['DeflateRaw', 'deflateRaw'],
    ['InflateRaw', 'inflateRaw'],
    ['Unzip', 'unzip'],
    ['BrotliCompress', 'brotliCompress'],
    ['BrotliDecompress', 'brotliDecompress'],
    ['ZstdCompress', 'zstdCompress'],
    ['ZstdDecompress', 'zstdDecompress'],
]) {
    const kind = KINDS[name];
    const Parent = kind >= KINDS.ZstdCompress ? Zstd : kind >= KINDS.BrotliCompress ? Brotli : Zlib;
    const Made = class extends Parent {
        constructor(options) {
            super(options, kind);
        }
    };
    Object.defineProperty(Made, 'name', { value: name });
    // What `info` gives back as the engine, which in Node did the work and was closed after.
    const engine = (options, input) => {
        const made = new Made(options);
        made.bytesWritten = input.length;
        made.close();
        return made;
    };
    exported[name] = Made;
    exported['create' + name] = (options) => new Made(options);
    exported[method + 'Sync'] = (data, options) => {
        const recipe = recipeOf(kind, options);
        const { finish, maxOutputLength } = optionsOf(kind, options);
        const input = inputOf(data);
        const out = __fs.zlib(input, finish, ...recipe);
        if (typeof out === 'string') {
            throw failure(out);
        }
        if (out.byteLength > maxOutputLength) {
            throw tooLarge(maxOutputLength);
        }
        const buffer = bufferOf(out);
        return options?.info ? { buffer, engine: engine(options, input) } : buffer;
    };
    exported[method] = (data, options, callback) => {
        if (typeof options === 'function') {
            callback = options;
            options = {};
        }
        if (typeof callback !== 'function') {
            throw Object.assign(new TypeError('The "callback" argument must be of type function. Received ' + typeof callback), { code: 'ERR_INVALID_ARG_TYPE' });
        }
        const recipe = recipeOf(kind, options);
        const { finish, maxOutputLength } = optionsOf(kind, options);
        const input = inputOf(data);
        __fs.zlibLater(input, finish, ...recipe).then(
            (out) => {
                if (out.byteLength > maxOutputLength) {
                    callback(tooLarge(maxOutputLength));
                    return;
                }
                const buffer = require('internal/buffer').Buffer.from(out.buffer, out.byteOffset, out.byteLength);
                callback(null, options?.info ? { buffer, engine: engine(options, input) } : buffer);
            },
            (error) => callback(failure(error.message)),
        );
    };
}

// Node still has zlib's own names on the module itself, but they aren't listed.
for (const [name, value] of Object.entries(constants)) {
    if (name.startsWith('Z_')) {
        Object.defineProperty(exported, name, { value, enumerable: false });
    }
}

module.exports = exported;
