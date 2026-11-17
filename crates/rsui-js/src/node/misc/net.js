'use strict';
// Node's `net`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = () => {
    const IPV4 = /^(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)$/;
    const isIPv4 = (text) => IPV4.test(String(text));
    const isIPv6 = (text) => {
        try {
            return String(text).includes(':') && new URL(`http://[${text}]`).hostname !== '';
        } catch {
            return false;
        }
    };
    const unavailable = () => {
        throw Object.assign(new Error('rsui has no sockets of its own. Use fetch for asking for things.'), { code: 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM' });
    };
    class Socket extends require('stream').Duplex {
        connect() {
            unavailable();
        }
    }
    return { isIP: (text) => (isIPv4(text) ? 4 : isIPv6(text) ? 6 : 0), isIPv4, isIPv6, Socket, createConnection: unavailable, connect: unavailable, createServer: unavailable, Server: class {} };
};
