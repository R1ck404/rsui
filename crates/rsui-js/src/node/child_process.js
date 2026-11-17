'use strict';
// Running other programs. No console comes up for them, even from a program with a window of its own.

const EventEmitter = require('events');
const { Readable, Writable } = require('stream');

const toBuffer = (bytes) => {
    const { Buffer } = require('internal/buffer');
    return Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
};

function environmentOf(options) {
    return options?.env ? Object.entries(options.env).filter(([, value]) => value !== undefined).map(([name, value]) => [name, String(value)]) : null;
}

// As Node goes about it, a shell is given everything as one command.
function shellCommand(file, args, options) {
    return options?.shell ? [file, ...args].join(' ') : file;
}

class ChildProcess extends EventEmitter {
    constructor(file, args, options) {
        super();
        this.spawnfile = file;
        this.spawnargs = [file, ...args];
        this.exitCode = null;
        this.signalCode = null;
        this.killed = false;
        this.connected = false;
        this.stdin = null;
        this.stdout = null;
        this.stderr = null;
        let started;
        try {
            started = __fs.start(shellCommand(file, args, options), args, options?.cwd, environmentOf(options), !!options?.shell);
        } catch (error) {
            error.syscall = 'spawn ' + file;
            error.path = file;
            error.spawnargs = args;
            this.pid = undefined;
            queueMicrotask(() => this.emit('error', error));
            return;
        }
        this.pid = started.pid;
        this.number = started.number;
        const number = started.number;
        this.stdin = new Writable({
            write(chunk, encoding, done) {
                __fs.writeTo(number, chunk).then(() => done(), done);
            },
            final(done) {
                __fs.writeTo(number, null).then(() => done(), done);
            },
        });
        this.stdout = new Readable({ read() {} });
        this.stderr = new Readable({ read() {} });
        const pump = async (stream, errors) => {
            for (;;) {
                const bytes = await __fs.readFrom(number, errors);
                if (bytes === null) {
                    stream.push(null);
                    return;
                }
                stream.push(toBuffer(bytes));
            }
        };
        const reading = [pump(this.stdout, false), pump(this.stderr, true)];
        queueMicrotask(() => this.emit('spawn'));
        __fs.waitFor(number).then(async (code) => {
            this.exitCode = code;
            const signal = this.killed ? 'SIGTERM' : null;
            this.signalCode = signal;
            this.emit('exit', signal ? null : code, signal);
            await Promise.all(reading);
            // What's read has to get to whoever's listening before it's said to be over.
            const ended = (stream) => (stream.readableEnded || stream.listenerCount('data') === 0 ? Promise.resolve() : new Promise((resolve) => stream.once('end', resolve)));
            await Promise.all([ended(this.stdout), ended(this.stderr)]);
            __fs.forget(number);
            this.emit('close', signal ? null : code, signal);
        });
    }

    kill() {
        if (this.number === undefined || this.exitCode !== null) {
            return false;
        }
        this.killed = __fs.stop(this.number);
        return this.killed;
    }

    [Symbol.dispose]() {
        if (!this.killed) {
            this.kill();
        }
    }

    ref() {}

    unref() {}
}

function spawn(file, args, options) {
    if (!Array.isArray(args)) {
        options = args;
        args = [];
    }
    return new ChildProcess(file, args.map(String), options);
}

function finishedError(command, code, signal, stdout, stderr) {
    const error = new Error(`Command failed: ${command}\n${stderr}`);
    error.code = code;
    error.killed = signal !== null;
    error.signal = signal;
    error.cmd = command;
    error.stdout = stdout;
    error.stderr = stderr;
    return error;
}

const promisifyCustom = Symbol.for('nodejs.util.promisify.custom');

function collect(child, command, options, done) {
    const encoding = options?.encoding === undefined ? 'utf8' : options.encoding;
    const parts = { stdout: [], stderr: [] };
    const textOf = (name) => {
        const { Buffer } = require('internal/buffer');
        const all = Buffer.concat(parts[name]);
        return encoding === 'buffer' || encoding === null ? all : all.toString(encoding);
    };
    child.stdout?.on('data', (chunk) => parts.stdout.push(chunk));
    child.stderr?.on('data', (chunk) => parts.stderr.push(chunk));
    let timer = null;
    if (options?.timeout > 0) {
        timer = setTimeout(() => child.kill(), options.timeout);
    }
    let told = false;
    const tell = (error, stdout, stderr) => {
        if (!told) {
            told = true;
            clearTimeout(timer);
            done?.(error, stdout, stderr);
        }
    };
    child.once('error', (error) => {
        error.cmd = command;
        tell(error, '', '');
    });
    child.once('close', (code, signal) => {
        const [stdout, stderr] = [textOf('stdout'), textOf('stderr')];
        tell(code === 0 ? null : finishedError(command, code, signal, stdout, stderr), stdout, stderr);
    });
    return child;
}

function exec(command, options, done) {
    if (typeof options === 'function') {
        done = options;
        options = undefined;
    }
    return collect(new ChildProcess(command, [], { ...options, shell: true }), command, options, done);
}

function execFile(file, args, options, done) {
    if (typeof args === 'function') {
        done = args;
        args = [];
        options = undefined;
    } else if (!Array.isArray(args)) {
        done = options;
        options = args;
        args = [];
    }
    if (typeof options === 'function') {
        done = options;
        options = undefined;
    }
    return collect(new ChildProcess(file, args.map(String), options), [file, ...args].join(' '), options, done);
}

for (const [run, makes] of [[exec, (...given) => exec(...given)], [execFile, (...given) => execFile(...given)]]) {
    run[promisifyCustom] = (...given) => {
        let child;
        const promise = new Promise((resolve, reject) => {
            child = makes(...given, (error, stdout, stderr) => (error ? reject(error) : resolve({ stdout, stderr })));
        });
        promise.child = child;
        return promise;
    };
}

function spawnSync(file, args, options) {
    if (!Array.isArray(args)) {
        options = args;
        args = [];
    }
    const given = options?.input;
    const input = given === undefined || given === null ? null : typeof given === 'string' ? new TextEncoder().encode(given) : given;
    const encoding = options?.encoding;
    const shaped = (bytes) => {
        const buffer = toBuffer(bytes);
        return encoding && encoding !== 'buffer' ? buffer.toString(encoding) : buffer;
    };
    try {
        const done = __fs.runNow(shellCommand(file, args, options), args.map(String), options?.cwd, environmentOf(options), !!options?.shell, input);
        const [stdout, stderr] = [shaped(done.stdout), shaped(done.stderr)];
        return { pid: 0, status: done.status, signal: null, stdout, stderr, output: [null, stdout, stderr] };
    } catch (error) {
        error.syscall = 'spawnSync ' + file;
        error.path = file;
        return { pid: 0, status: null, signal: null, stdout: null, stderr: null, output: null, error };
    }
}

function outputOr(done, command) {
    if (done.error) {
        throw done.error;
    }
    if (done.status !== 0) {
        throw Object.assign(finishedError(command, done.status, null, done.stdout, done.stderr), { status: done.status, output: done.output });
    }
    return done.stdout;
}

function execSync(command, options) {
    return outputOr(spawnSync(command, [], { ...options, shell: true }), command);
}

function execFileSync(file, args, options) {
    if (!Array.isArray(args)) {
        options = args;
        args = [];
    }
    return outputOr(spawnSync(file, args, options), [file, ...args].join(' '));
}

module.exports = { ChildProcess, spawn, exec, execFile, spawnSync, execSync, execFileSync };
