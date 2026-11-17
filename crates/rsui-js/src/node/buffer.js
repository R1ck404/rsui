'use strict';
// `Buffer`, as bytes that know how to turn into text and back. It's only compiled when something looks for it.

// Each encoding by the number the natives in `builtins/host/buffer.rs` know it by, going to bytes and coming back.
const TO_BYTES = new Map([['utf8', 0], ['utf-8', 0], ['hex', 1], ['base64', 2], ['base64url', 2], ['latin1', 3], ['binary', 3], ['ascii', 3], ['utf16le', 4], ['utf-16le', 4], ['ucs2', 4], ['ucs-2', 4]]);
const TO_TEXT = new Map([['utf8', 0], ['utf-8', 0], ['hex', 1], ['base64', 2], ['base64url', 3], ['latin1', 4], ['binary', 4], ['ascii', 5], ['utf16le', 6], ['utf-16le', 6], ['ucs2', 6], ['ucs-2', 6]]);

const unknownEncoding = (encoding) => Object.assign(new TypeError('Unknown encoding: ' + encoding), { code: 'ERR_UNKNOWN_ENCODING' });

// Text's bytes, in an `ArrayBuffer` of their own.
function arrayBufferOf(text, encoding = 'utf8') {
    const kind = TO_BYTES.get(String(encoding).toLowerCase());
    if (kind === undefined) {
        throw unknownEncoding(encoding);
    }
    return __fs.bufferFrom(text, kind);
}

const bytesOf = (text, encoding) => new Uint8Array(arrayBufferOf(text, encoding));

// `Buffer` itself is a plain function, as what's written for Node can call it without `new`, or give it what comes
// from `FastBuffer`, which is what these really are.
class FastBuffer extends Uint8Array {
    toString(encoding = 'utf8', start = 0, end = this.length) {
        const part = start === 0 && end === this.length ? this : this.subarray(Math.max(0, start), Math.min(this.length, end));
        const kind = TO_TEXT.get(String(encoding ?? 'utf8').toLowerCase());
        if (kind === undefined) {
            throw unknownEncoding(encoding);
        }
        return __fs.bufferText(part, kind);
    }
    toLocaleString(encoding, start, end) {
        return this.toString(encoding, start, end);
    }
    equals(other) {
        if (this.length !== other.length) {
            return false;
        }
        const order = __fs.bufferCompare(this, other);
        return order === undefined ? this.every((byte, at) => byte === other[at]) : order === 0;
    }
    compare(other, start = 0, end = other.length, ownStart = 0, ownEnd = this.length) {
        return compareBytes(this.subarray(ownStart, ownEnd), other.subarray(start, end));
    }
    copy(target, at = 0, start = 0, end = this.length) {
        const part = this.subarray(start, Math.min(end, start + target.length - at));
        target.set(part, at);
        return part.length;
    }
    slice(start, end) {
        return this.subarray(start, end);
    }
    fill(value, offset, end, encoding) {
        if (typeof offset === 'string') {
            encoding = offset;
            offset = undefined;
        } else if (typeof end === 'string') {
            encoding = end;
            end = undefined;
        }
        const [from, to] = [offset ?? 0, end ?? this.length];
        if (typeof value === 'string' || ArrayBuffer.isView(value)) {
            const bytes = typeof value === 'string' ? bytesOf(value, encoding) : value;
            if (bytes.length === 0) {
                return super.fill(0, from, to);
            }
            for (let at = from; at < to; at += 1) this[at] = bytes[(at - from) % bytes.length];
            return this;
        }
        return super.fill(value & 255, from, to);
    }
    indexOf(value, from = 0, encoding) {
        const wanted = typeof value === 'number' ? Uint8Array.of(value & 255) : typeof value === 'string' ? bytesOf(value, encoding) : value;
        const first = from < 0 ? Math.max(0, this.length + from) : from;
        if (Number.isInteger(first) && first >= 0) {
            const found = __fs.bufferIndexOf(this, wanted, first, false);
            if (found !== undefined) {
                return found;
            }
        }
        outer: for (let at = first; at <= this.length - wanted.length; at += 1) {
            for (let inner = 0; inner < wanted.length; inner += 1) {
                if (this[at + inner] !== wanted[inner]) continue outer;
            }
            return at;
        }
        return -1;
    }
    lastIndexOf(value, from = this.length, encoding) {
        const wanted = typeof value === 'number' ? Uint8Array.of(value & 255) : typeof value === 'string' ? bytesOf(value, encoding) : value;
        if (Number.isInteger(from) && from >= 0) {
            const found = __fs.bufferIndexOf(this, wanted, from, true);
            if (found !== undefined) {
                return found;
            }
        }
        outer: for (let at = Math.min(from, this.length - wanted.length); at >= 0; at -= 1) {
            for (let inner = 0; inner < wanted.length; inner += 1) {
                if (this[at + inner] !== wanted[inner]) continue outer;
            }
            return at;
        }
        return -1;
    }
    includes(value, from, encoding) {
        return this.indexOf(value, from, encoding) !== -1;
    }
    write(text, offset, length, encoding) {
        if (typeof offset === 'string') {
            encoding = offset;
            offset = 0;
        } else if (typeof length === 'string') {
            encoding = length;
            length = undefined;
        }
        const at = offset ?? 0;
        const room = Math.min(length ?? this.length - at, this.length - at);
        const bytes = bytesOf(String(text), encoding).subarray(0, room);
        this.set(bytes, at);
        return bytes.length;
    }
    toJSON() {
        return { type: 'Buffer', data: Array.from(this) };
    }
    swap16() {
        for (let at = 0; at + 1 < this.length; at += 2) [this[at], this[at + 1]] = [this[at + 1], this[at]];
        return this;
    }
    swap32() {
        for (let at = 0; at + 3 < this.length; at += 4) this.subarray(at, at + 4).reverse();
        return this;
    }
    swap64() {
        for (let at = 0; at + 7 < this.length; at += 8) this.subarray(at, at + 8).reverse();
        return this;
    }
    get parent() {
        return this.buffer;
    }
    [Symbol.for('nodejs.util.inspect.custom')]() {
        const shown = [];
        for (let at = 0; at < Math.min(50, this.length); at += 1) {
            shown.push(this[at].toString(16).padStart(2, '0'));
        }
        const more = this.length - 50;
        return `<Buffer${shown.length > 0 ? ' ' + shown.join(' ') : ''}${more > 0 ? ` ... ${more} more byte${more === 1 ? '' : 's'}` : ''}>`;
    }
}

function compareBytes(a, b) {
    const order = __fs.bufferCompare(a, b);
    if (order !== undefined) {
        return order;
    }
    const shared = Math.min(a.length, b.length);
    for (let at = 0; at < shared; at += 1) {
        if (a[at] !== b[at]) return a[at] < b[at] ? -1 : 1;
    }
    return a.length === b.length ? 0 : a.length < b.length ? -1 : 1;
}

function checkRange(buffer, offset, size) {
    if (!Number.isInteger(offset)) {
        throw Object.assign(new RangeError('The value of "offset" is out of range. It must be an integer.'), { code: 'ERR_OUT_OF_RANGE' });
    }
    if (offset < 0 || offset + size > buffer.length) {
        if (buffer.length - size < 0) {
            throw Object.assign(new RangeError('Attempt to access memory outside buffer bounds'), { code: 'ERR_BUFFER_OUT_OF_BOUNDS' });
        }
        throw Object.assign(new RangeError(`The value of "offset" is out of range. It must be >= 0 and <= ${buffer.length - size}. Received ${offset}`), { code: 'ERR_OUT_OF_RANGE' });
    }
}

const viewOf = (buffer) => new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength);

// Reading and writing numbers, which is a lot of names for not much.
for (const [names, size, get, set] of [
    [['UInt8', 'Uint8'], 1, 'getUint8', 'setUint8'],
    [['Int8'], 1, 'getInt8', 'setInt8'],
    [['UInt16', 'Uint16'], 2, 'getUint16', 'setUint16'],
    [['Int16'], 2, 'getInt16', 'setInt16'],
    [['UInt32', 'Uint32'], 4, 'getUint32', 'setUint32'],
    [['Int32'], 4, 'getInt32', 'setInt32'],
    [['Float'], 4, 'getFloat32', 'setFloat32'],
    [['Double'], 8, 'getFloat64', 'setFloat64'],
    [['BigInt64'], 8, 'getBigInt64', 'setBigInt64'],
    [['BigUInt64', 'BigUint64'], 8, 'getBigUint64', 'setBigUint64'],
]) {
    for (const name of names) {
        for (const [ending, little] of size === 1 ? [['', true]] : [['LE', true], ['BE', false]]) {
            FastBuffer.prototype['read' + name + ending] = function (offset = 0) {
                checkRange(this, offset, size);
                return viewOf(this)[get](offset, little);
            };
            FastBuffer.prototype['write' + name + ending] = function (value, offset = 0) {
                checkRange(this, offset, size);
                viewOf(this)[set](offset, value, little);
                return offset + size;
            };
        }
    }
}

for (const [name, signed] of [['UInt', false], ['Uint', false], ['Int', true]]) {
    for (const [ending, little] of [['LE', true], ['BE', false]]) {
        FastBuffer.prototype[`read${name}${ending}`] = function (offset, size) {
            checkRange(this, offset, size);
            let value = 0;
            for (let step = 0; step < size; step += 1) {
                value = value * 256 + this[little ? offset + size - 1 - step : offset + step];
            }
            return signed && value >= 2 ** (size * 8 - 1) ? value - 2 ** (size * 8) : value;
        };
        FastBuffer.prototype[`write${name}${ending}`] = function (value, offset, size) {
            checkRange(this, offset, size);
            let rest = value < 0 ? value + 2 ** (size * 8) : value;
            for (let step = 0; step < size; step += 1) {
                this[little ? offset + step : offset + size - 1 - step] = rest % 256;
                rest = Math.floor(rest / 256);
            }
            return offset + size;
        };
    }
}

// The older names for turning part of one into text, which are still what some of it looks for.
for (const [name, encoding] of [['utf8', 'utf8'], ['latin1', 'latin1'], ['ascii', 'ascii'], ['hex', 'hex'], ['base64', 'base64'], ['ucs2', 'utf16le']]) {
    FastBuffer.prototype[name + 'Slice'] = function (start, end) {
        return this.toString(encoding, start, end);
    };
    FastBuffer.prototype[name + 'Write'] = function (text, offset, length) {
        return this.write(text, offset, length, encoding);
    };
}

function Buffer(value, second, third) {
    if (typeof value === 'number') {
        return new FastBuffer(value);
    }
    return Buffer.from(value, second, third);
}
Buffer.prototype = FastBuffer.prototype;
Object.defineProperty(FastBuffer.prototype, 'constructor', { value: Buffer, writable: true, configurable: true });
Object.setPrototypeOf(Buffer, Uint8Array);
Object.assign(Buffer, {
    poolSize: 8192,
    from(value, second, third) {
        if (typeof value === 'string') {
            return new FastBuffer(arrayBufferOf(value, second));
        }
        if (value instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && value instanceof SharedArrayBuffer)) {
            return new FastBuffer(value, second, third);
        }
        if (value === null || value === undefined) {
            throw Object.assign(new TypeError('The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object.'), { code: 'ERR_INVALID_ARG_TYPE' });
        }
        if (typeof value === 'object' && value.type === 'Buffer' && Array.isArray(value.data)) {
            return Buffer.from(value.data);
        }
        const primitive = value.valueOf?.();
        if (primitive !== undefined && primitive !== null && primitive !== value) {
            return Buffer.from(primitive, second, third);
        }
        if (typeof value.length !== 'number') {
            if (typeof value[Symbol.toPrimitive] === 'function') {
                return Buffer.from(value[Symbol.toPrimitive]('string'), second, third);
            }
            throw Object.assign(new TypeError('The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object.'), { code: 'ERR_INVALID_ARG_TYPE' });
        }
        const out = new FastBuffer(value.length);
        out.set(value);
        return out;
    },
    alloc(size, fill, encoding) {
        const out = new FastBuffer(size);
        return fill === undefined || fill === 0 ? out : out.fill(fill, encoding);
    },
    allocUnsafe: (size) => new FastBuffer(size),
    allocUnsafeSlow: (size) => new FastBuffer(size),
    isBuffer: (value) => value instanceof FastBuffer,
    isEncoding: (name) => typeof name === 'string' && ['utf8', 'utf-8', 'hex', 'base64', 'base64url', 'latin1', 'binary', 'ascii', 'utf16le', 'utf-16le', 'ucs2', 'ucs-2'].includes(name.toLowerCase()),
    byteLength: (value, encoding) => (typeof value === 'string' ? bytesOf(value, encoding).length : value.byteLength),
    compare: compareBytes,
    concat(list, total) {
        const size = total ?? list.reduce((sum, item) => sum + item.length, 0);
        const out = new FastBuffer(size);
        let at = 0;
        for (const item of list) {
            if (at >= size) break;
            out.set(item.subarray(0, size - at), at);
            at += item.length;
        }
        return out;
    },
});

module.exports = { Buffer };
