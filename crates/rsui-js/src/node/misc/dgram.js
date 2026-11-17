'use strict';
// Node's `dgram`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = () => {
    return {
        createSocket() {
            throw Object.assign(new Error('rsui has no sockets of its own.'), { code: 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM' });
        },
    };
};
