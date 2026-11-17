'use strict';
// The small modules of Node. Each is in `node/misc/`, and only compiled the first time it's asked for, as most programs
// only want a few of them.
const makers = {};
for (const name of ['os', 'crypto', 'querystring', 'string_decoder', 'timers', 'module', 'punycode', 'tty', 'net', 'dns', 'worker_threads', 'dgram', 'perf_hooks']) {
    makers[name] = () => require('internal/misc/' + name)(__fs);
}

module.exports = (name) => makers[name]();
