'use strict';
// Node's `dns`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = () => {
    const unavailable = (name, ...given) => {
        const done = given[given.length - 1];
        const error = Object.assign(new Error(`getaddrinfo ENOTSUP ${name}`), { code: 'ENOTSUP' });
        if (typeof done === 'function') {
            queueMicrotask(() => done(error));
            return undefined;
        }
        return Promise.reject(error);
    };
    return { lookup: unavailable, resolve: unavailable, resolve4: unavailable, resolve6: unavailable, promises: { lookup: unavailable, resolve: unavailable } };
};
