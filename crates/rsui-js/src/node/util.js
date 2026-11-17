'use strict';
// Node's `util`, for packages that expect it. It's only compiled if something asks for it.
const inspect = (value, options) => __inspect(value);
inspect.custom = Symbol.for('nodejs.util.inspect.custom');
inspect.defaultOptions = {};
function format(first, ...rest) {
    if (typeof first !== 'string') return [first, ...rest].map(item => typeof item === 'string' ? item : inspect(item)).join(' ');
    let at = 0;
    const out = first.replace(/%[sdifjoOc%]/g, token => {
        if (token === '%%') return '%';
        if (at >= rest.length) return token;
        const value = rest[at++];
        switch (token) {
            case '%s': return typeof value === 'string' ? value : inspect(value);
            case '%d': return String(Number(value));
            case '%i': return String(parseInt(value));
            case '%f': return String(parseFloat(value));
            case '%j': return JSON.stringify(value);
            case '%c': return '';
            default: return inspect(value);
        }
    });
    return [out, ...rest.slice(at).map(item => typeof item === 'string' ? item : inspect(item))].join(' ');
}
function isDeepStrictEqual(a, b) {
    if (Object.is(a, b)) return true;
    if (typeof a !== 'object' || typeof b !== 'object' || !a || !b) return false;
    if (Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;
    if (a instanceof Date) return a.getTime() === b.getTime();
    if (a instanceof RegExp) return String(a) === String(b);
    if (a instanceof Map) {
        if (a.size !== b.size) return false;
        for (const [k, v] of a) if (!b.has(k) || !isDeepStrictEqual(v, b.get(k))) return false;
        return true;
    }
    if (a instanceof Set) {
        if (a.size !== b.size) return false;
        for (const v of a) if (!b.has(v)) return false;
        return true;
    }
    const keys = Reflect.ownKeys(a);
    if (keys.length !== Reflect.ownKeys(b).length) return false;
    return keys.every(key => Object.hasOwn(b, key) && isDeepStrictEqual(a[key], b[key]));
}
module.exports = {
    inspect,
    format,
    isDeepStrictEqual,
    inherits(child, parent) {
        Object.defineProperty(child, 'super_', { value: parent, writable: true, configurable: true });
        Object.setPrototypeOf(child.prototype, parent.prototype);
    },
    deprecate: fn => fn,
    promisify: fn => fn[Symbol.for('nodejs.util.promisify.custom')] ?? ((...args) => new Promise((resolve, reject) => fn(...args, (error, value) => error ? reject(error) : resolve(value)))),
    callbackify: fn => (...args) => { const done = args.pop(); fn(...args).then(value => done(null, value), done); },
    debuglog: () => () => {},
    stripVTControlCharacters: text => text.replace(/\u001b\[[0-9;]*m/g, ''),
    TextEncoder: globalThis.TextEncoder,
    TextDecoder: globalThis.TextDecoder,
    types: {
        isPromise: v => v instanceof Promise,
        isDate: v => v instanceof Date,
        isRegExp: v => v instanceof RegExp,
        isMap: v => v instanceof Map,
        isSet: v => v instanceof Set,
        isTypedArray: v => ArrayBuffer.isView(v) && !(v instanceof DataView),
        isUint8Array: v => v instanceof Uint8Array,
        isAnyArrayBuffer: v => v instanceof ArrayBuffer,
        // Only a namespace is like this: nothing else can be made with no prototype, unable to grow, and saying it's a module.
        isModuleNamespaceObject: v => v !== null && typeof v === 'object' && Object.getPrototypeOf(v) === null && !Object.isExtensible(v) && Object.prototype.toString.call(v) === '[object Module]',
    },
    isArray: Array.isArray,
};
