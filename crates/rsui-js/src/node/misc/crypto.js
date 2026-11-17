'use strict';
// Node's `crypto`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = (__fs) => {
    const { Buffer } = require('internal/buffer');
    const bytesOf = (data, encoding) => (typeof data === 'string' ? Buffer.from(data, encoding ?? 'utf8') : data);
    // Inspect descriptors without calling conversion hooks. Buffer.from observes
    // inherited hooks too, including on the temporary Buffers made for strings.
    const typedPrototype = Object.getPrototypeOf(Uint8Array.prototype);
    const typedLength = Object.getOwnPropertyDescriptor(typedPrototype, 'length').get;
    const objectValueOf = Object.getOwnPropertyDescriptor(Object.prototype, 'valueOf').value;
    const noPbkdf2Hooks = (value, allowLength = false, allowValueOf = false) =>
        (allowValueOf || !Object.hasOwn(value, 'valueOf')) &&
        (allowLength || !Object.hasOwn(value, 'length')) &&
        !Object.hasOwn(value, 'type') && !Object.hasOwn(value, Symbol.toPrimitive);
    const ordinaryPbkdf2Prototypes = () =>
        Object.getPrototypeOf(Buffer.prototype) === Uint8Array.prototype &&
        Object.getPrototypeOf(Uint8Array.prototype) === typedPrototype &&
        Object.getPrototypeOf(typedPrototype) === Object.prototype &&
        noPbkdf2Hooks(Buffer.prototype) && noPbkdf2Hooks(Uint8Array.prototype) &&
        noPbkdf2Hooks(typedPrototype, true) && noPbkdf2Hooks(Object.prototype, false, true) &&
        Object.getOwnPropertyDescriptor(typedPrototype, 'length')?.get === typedLength &&
        Object.getOwnPropertyDescriptor(Object.prototype, 'valueOf')?.value === objectValueOf;
    const ordinaryPbkdf2Input = (value) => typeof value === 'string' ||
        (ArrayBuffer.isView(value) && value instanceof Uint8Array &&
            (Object.getPrototypeOf(value) === Uint8Array.prototype || Object.getPrototypeOf(value) === Buffer.prototype) &&
            noPbkdf2Hooks(value));
    const BLOCK = { md5: 64, sha1: 64, sha256: 64, sha384: 128, sha512: 128 };
    const nameOf = (name) => String(name).toLowerCase().replace('-', '');
    const output = (bytes, encoding) => {
        const buffer = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        return encoding ? buffer.toString(encoding) : buffer;
    };
    const digestOf = (name, bytes) => {
        const found = __fs.hash(name, bytes);
        if (found === undefined) {
            throw new Error('Digest method not supported');
        }
        return found;
    };
    class Hash {
        constructor(name) {
            this.name = nameOf(name);
            digestOf(this.name, new Uint8Array(0));
            this.parts = [];
            this.done = false;
        }
        update(data, encoding) {
            if (this.done) {
                throw Object.assign(new Error('Digest already called'), { code: 'ERR_CRYPTO_HASH_FINALIZED' });
            }
            this.parts.push(Buffer.from(bytesOf(data, encoding)));
            return this;
        }
        digest(encoding) {
            if (this.done) {
                throw Object.assign(new Error('Digest already called'), { code: 'ERR_CRYPTO_HASH_FINALIZED' });
            }
            this.done = true;
            return output(digestOf(this.name, Buffer.concat(this.parts)), encoding);
        }
    }
    class Hmac extends Hash {
        constructor(name, key) {
            super(name);
            const block = BLOCK[this.name];
            let given = key instanceof KeyObject ? key.bytes : Buffer.from(bytesOf(key));
            if (given.length > block) {
                given = Buffer.from(digestOf(this.name, given));
            }
            const padded = (value) => {
                const out = Buffer.alloc(block, value);
                given.forEach((byte, at) => {
                    out[at] = byte ^ value;
                });
                return out;
            };
            this.inner = padded(0x36);
            this.outer = padded(0x5c);
        }
        digest(encoding) {
            if (this.done) {
                throw Object.assign(new Error('Digest already called'), { code: 'ERR_CRYPTO_HASH_FINALIZED' });
            }
            this.done = true;
            const first = digestOf(this.name, Buffer.concat([this.inner, ...this.parts]));
            return output(digestOf(this.name, Buffer.concat([this.outer, first])), encoding);
        }
    }
    // A key that's only a secret is only bytes. There's nothing here to do with public and private ones, and what
    // would use them says so when it's called.
    class KeyObject {
        constructor(bytes) {
            this.bytes = bytes;
            this.type = 'secret';
            this.symmetricKeySize = bytes.length;
        }
        export() {
            return this.bytes;
        }
        equals(other) {
            return other instanceof KeyObject && this.bytes.equals(other.bytes);
        }
    }
    const noKeys = (name) => () => {
        throw Object.assign(new Error(`${name} isn't in rsui, which has hashes and random numbers but no public keys or ciphers.`), { code: 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM' });
    };
    const randomBytes = (size, done) => {
        const made = Buffer.from(globalThis.crypto.getRandomValues(new Uint8Array(size)).buffer);
        if (done) {
            queueMicrotask(() => done(null, made));
            return undefined;
        }
        return made;
    };
    return {
        createHash: (name) => new Hash(name),
        createHmac: (name, key) => new Hmac(name, key),
        KeyObject,
        createSecretKey: (key, encoding) => new KeyObject(Buffer.from(bytesOf(key, encoding))),
        createPublicKey: noKeys('createPublicKey'),
        createPrivateKey: noKeys('createPrivateKey'),
        createSign: noKeys('createSign'),
        createVerify: noKeys('createVerify'),
        createCipheriv: noKeys('createCipheriv'),
        createDecipheriv: noKeys('createDecipheriv'),
        publicEncrypt: noKeys('publicEncrypt'),
        privateDecrypt: noKeys('privateDecrypt'),
        pbkdf2Sync(password, salt, rounds, length, digest) {
            // Only ordinary inputs and checked primitive numbers use the native path.
            // Other forms keep the existing coercion and validation order below.
            if (typeof digest === 'string' && Object.hasOwn(BLOCK, nameOf(digest)) &&
                Number.isInteger(rounds) && rounds > 0 && rounds <= 0x7fffffff &&
                Number.isInteger(length) && length >= 0 && length <= 0x7fffffff &&
                ordinaryPbkdf2Prototypes() &&
                ordinaryPbkdf2Input(password) && ordinaryPbkdf2Input(salt)) {
                return output(__fs.pbkdf2(bytesOf(password), bytesOf(salt), rounds, length, nameOf(digest)));
            }
            const block = digestOf(nameOf(digest), new Uint8Array(0)).length;
            const key = Buffer.from(bytesOf(password));
            const out = Buffer.alloc(length);
            for (let index = 1, at = 0; at < length; index += 1, at += block) {
                const count = Buffer.alloc(4);
                count.writeUInt32BE(index);
                let u = new Hmac(digest, key).update(bytesOf(salt)).update(count).digest();
                const t = Buffer.from(u);
                for (let round = 1; round < rounds; round += 1) {
                    u = new Hmac(digest, key).update(u).digest();
                    for (let place = 0; place < block; place += 1) {
                        t[place] ^= u[place];
                    }
                }
                t.copy(out, at);
            }
            return out;
        },
        pbkdf2(password, salt, rounds, length, digest, done) {
            queueMicrotask(() => {
                let made;
                try {
                    made = this.pbkdf2Sync(password, salt, rounds, length, digest);
                } catch (error) {
                    done(error);
                    return;
                }
                done(null, made);
            });
        },
        hash: (name, data, encoding = 'hex') => output(digestOf(nameOf(name), bytesOf(data)), encoding),
        getHashes: () => Object.keys(BLOCK),
        randomBytes,
        randomFillSync: (target) => globalThis.crypto.getRandomValues(target),
        randomUUID: () => globalThis.crypto.randomUUID(),
        getRandomValues: (target) => globalThis.crypto.getRandomValues(target),
        randomInt(low, high, done) {
            if (high === undefined || typeof high === 'function') {
                done = high;
                high = low;
                low = 0;
            }
            const made = low + Math.floor((globalThis.crypto.getRandomValues(new Uint32Array(1))[0] / 2 ** 32) * (high - low));
            if (done) {
                queueMicrotask(() => done(null, made));
                return undefined;
            }
            return made;
        },
        timingSafeEqual(a, b) {
            if (a.byteLength !== b.byteLength) {
                throw Object.assign(new RangeError('Input buffers must have the same byte length'), { code: 'ERR_CRYPTO_TIMING_SAFE_EQUAL_LENGTH' });
            }
            let different = 0;
            for (let at = 0; at < a.length; at += 1) {
                different |= a[at] ^ b[at];
            }
            return different === 0;
        },
        webcrypto: globalThis.crypto,
    };
};
