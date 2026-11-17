'use strict';
// Node's `perf_hooks`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = () => {
    return { performance: globalThis.performance };
};
