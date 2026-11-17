'use strict';
// `process.stdout` and the others, which are only made if they're looked for.

const { Readable, Writable } = require('stream');

function out(errors) {
    const stream = new Writable({
        decodeStrings: false,
        write(chunk, encoding, done) {
            __fs.writeOut(typeof chunk === 'string' ? chunk : new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength), errors);
            done();
        },
    });
    // These stay open, and can be written to whatever's happened.
    stream.autoDestroy = false;
    stream.isTTY = false;
    stream.fd = errors ? 2 : 1;
    stream.columns = 80;
    return stream;
}

// There's nothing to read from a program with a window, and nothing to wait for.
const stdin = new Readable({ read() { this.push(null); } });
stdin.fd = 0;
stdin.isTTY = false;

module.exports = { stdout: out(false), stderr: out(true), stdin };
