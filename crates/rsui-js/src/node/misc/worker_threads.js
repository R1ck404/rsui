'use strict';
// Node's `worker_threads`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = () => {
    return {
        isMainThread: true,
        parentPort: null,
        workerData: null,
        threadId: 0,
        Worker: class {
            constructor() {
                throw Object.assign(new Error('rsui has no worker threads.'), { code: 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM' });
            }
        },
        MessageChannel: globalThis.MessageChannel,
    };
};
