'use strict';
// Node's `tty`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = () => {
    const stream = require('stream');
    return { isatty: () => false, ReadStream: stream.Readable, WriteStream: stream.Writable };
};
