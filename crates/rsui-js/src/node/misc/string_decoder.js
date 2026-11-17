'use strict';
// Node's `string_decoder`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = () => {
    const { Buffer } = require('internal/buffer');
    class StringDecoder {
        constructor(encoding = 'utf8') {
            this.encoding = String(encoding).toLowerCase().replace('-', '');
            this.left = Buffer.alloc(0);
        }
        write(chunk) {
            if (typeof chunk === 'string') {
                return chunk;
            }
            const all = Buffer.concat([this.left, Buffer.from(chunk)]);
            if (this.encoding === 'utf8') {
                // What's on the end might be the first part of a character, whose other part comes next.
                let keep = 0;
                for (let back = 1; back <= Math.min(3, all.length); back += 1) {
                    const byte = all[all.length - back];
                    if ((byte & 0xc0) === 0x80) {
                        continue;
                    }
                    const needs = byte >= 0xf0 ? 4 : byte >= 0xe0 ? 3 : byte >= 0xc0 ? 2 : 1;
                    if (needs > back) {
                        keep = back;
                    }
                    break;
                }
                this.left = all.subarray(all.length - keep);
                return all.subarray(0, all.length - keep).toString('utf8');
            }
            if (this.encoding === 'utf16le' || this.encoding === 'ucs2') {
                const keep = all.length % 2;
                this.left = all.subarray(all.length - keep);
                return all.subarray(0, all.length - keep).toString('utf16le');
            }
            if (this.encoding === 'base64') {
                const keep = all.length % 3;
                this.left = all.subarray(all.length - keep);
                return all.subarray(0, all.length - keep).toString('base64');
            }
            this.left = Buffer.alloc(0);
            return all.toString(this.encoding);
        }
        end(chunk) {
            const before = chunk === undefined ? '' : this.write(chunk);
            const rest = this.left.length > 0 ? this.left.toString(this.encoding) : '';
            this.left = Buffer.alloc(0);
            return before + rest;
        }
    }
    return { StringDecoder };
};
