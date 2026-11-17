'use strict';
// Node's `os`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = (__fs) => {
    const machine = () => __fs.machine();
    const env = process.env;
    return {
        EOL: '\r\n',
        devNull: '\\\\.\\nul',
        arch: () => 'x64',
        machine: () => 'x86_64',
        platform: () => 'win32',
        type: () => 'Windows_NT',
        release: () => machine().release,
        version: () => 'Windows',
        endianness: () => 'LE',
        hostname: () => env.COMPUTERNAME ?? 'localhost',
        homedir: () => env.USERPROFILE ?? 'C:\\',
        tmpdir: () => (env.TEMP ?? env.TMP ?? 'C:\\Windows\\Temp').replace(/\\$/, ''),
        totalmem: () => machine().totalmem,
        freemem: () => machine().freemem,
        availableParallelism: () => machine().cpus,
        uptime: () => machine().uptime,
        loadavg: () => [0, 0, 0],
        networkInterfaces: () => ({}),
        cpus: () => Array.from({ length: machine().cpus }, () => ({ model: 'unknown', speed: 0, times: { user: 0, nice: 0, sys: 0, idle: 0, irq: 0 } })),
        userInfo: () => ({ uid: -1, gid: -1, username: env.USERNAME ?? 'user', homedir: env.USERPROFILE ?? 'C:\\', shell: null }),
        constants: { signals: { SIGINT: 2, SIGTERM: 15, SIGKILL: 9 }, errno: {} },
    };
};
