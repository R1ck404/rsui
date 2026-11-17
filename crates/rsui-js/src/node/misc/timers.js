'use strict';
// Node's `timers`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = () => {
    const wait = (milliseconds, value) => new Promise((resolve) => setTimeout(resolve, milliseconds, value));
    return {
        setTimeout,
        clearTimeout,
        setInterval,
        clearInterval,
        setImmediate,
        clearImmediate,
        promises: { setTimeout: wait, setImmediate: (value) => wait(0, value) },
    };
};
