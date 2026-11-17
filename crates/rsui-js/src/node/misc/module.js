'use strict';
// Node's `module`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = () => {
    // Each module of Node's the engine has, as Node lists them. What only a server or Node itself has a use for, such as `cluster`,
    // `http2`, `vm` and `readline`, isn't here.
    const builtinModules = [
        'assert', 'assert/strict', 'buffer', 'child_process', 'console', 'constants', 'crypto', 'dgram', 'dns', 'dns/promises', 'events',
        'fs', 'fs/promises', 'http', 'https', 'module', 'net', 'os', 'path', 'path/posix', 'path/win32', 'perf_hooks', 'process', 'punycode',
        'querystring', 'stream', 'stream/promises', 'string_decoder', 'timers', 'timers/promises', 'tty', 'url', 'util', 'util/types',
        'worker_threads', 'zlib',
    ];
    const Module = function Module() {};
    return Object.assign(Module, {
        builtinModules,
        isBuiltin: (name) => builtinModules.includes(String(name).replace(/^node:/, '')),
        createRequire,
    });
};

/// A `require` that finds what it's given from where `file` is, which is a path or a `file:` URL.
function createRequire(file) {
    const given = typeof file === 'string' ? file : String(file?.href ?? file);
    const full = given.startsWith('file:') ? require('url').fileURLToPath(given) : given;
    const path = require('path');
    // A folder is written with a slash on its end, and what's inside it is found from there.
    const folder = /[\\/]$/.test(full) ? full : path.dirname(full);
    const resolve = (name, options) => require.resolve(name, options ?? { paths: [folder] });
    const made = (name) => require(resolve(name));
    made.resolve = resolve;
    made.cache = Object.create(null);
    return made;
}
