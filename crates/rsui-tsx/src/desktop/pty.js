// `pty` in `rsui/desktop`: a program run in a pseudo terminal, for an app that draws a terminal. What it prints comes as text with
// its escape sequences in it, and what's typed goes in as text. While an app's being checked nothing runs: it's pretended, and the
// steps `pty out TEXT` and `pty exit` make the program print and end. `RSUI_PTY=real` runs them for real even then.

const { host, action, forget } = require('rsui/desktop/core');

class Pty {
    #id;
    #actions;
    #size;
    #exited = false;

    constructor(id, actions, size) {
        this.#id = id;
        this.#actions = actions;
        this.#size = size;
    }

    /// The program's number in the system, or 0 while it's pretended.
    get pid() {
        return this.#exited ? 0 : host.ptyPid(this.#id);
    }

    get cols() {
        return this.#size[0];
    }

    get rows() {
        return this.#size[1];
    }

    get exited() {
        return this.#exited;
    }

    /// Types this, as the keyboard would.
    write(text) {
        if (!this.#exited && text.length > 0) {
            host.ptyWrite(this.#id, text);
        }
    }

    /// Tells the program how big its terminal is now.
    resize(cols, rows) {
        cols = Math.max(1, Math.floor(cols));
        rows = Math.max(1, Math.floor(rows));
        if (cols === this.#size[0] && rows === this.#size[1]) {
            return;
        }
        this.#size = [cols, rows];
        if (!this.#exited) {
            host.ptyResize(this.#id, cols, rows);
        }
    }

    /// Ends the program. `onExit` still runs.
    kill() {
        if (!this.#exited) {
            host.ptyKill(this.#id);
        }
    }

    /// Lets go of it: the program ends if it's still running, and nothing more is told.
    dispose() {
        forget(this.#actions);
        host.ptyFree(this.#id);
        this.#exited = true;
    }

    _ended() {
        this.#exited = true;
    }
}

const pty = {
    /// Starts `command` (a command line) in a terminal of `cols` by `rows`. `onData` runs with what it prints and `onExit` with its
    /// exit code once that's all been given. `env` is added to what it inherits.
    spawn({ command, cwd, env, cols = 80, rows = 24, onData, onExit }) {
        if (typeof command !== 'string' || command === '') {
            throw new TypeError('pty.spawn needs a command line');
        }
        let id = 0;
        let made = null;
        const dataAction = action((text) => {
            if (id !== 0) {
                host.ptyAck(id);
            }
            if (onData) {
                onData(text);
            }
        });
        const exitAction = action((code) => {
            made?._ended();
            if (onExit) {
                onExit(Number(code));
            }
            forget([dataAction, exitAction]);
        });
        const spec = JSON.stringify({ command, cwd: cwd ?? null, env: env ?? {}, cols, rows });
        try {
            id = host.ptySpawn(spec, dataAction, exitAction);
        } catch (error) {
            forget([dataAction, exitAction]);
            throw error;
        }
        made = new Pty(id, [dataAction, exitAction], [cols, rows]);
        return made;
    },
};

module.exports = { pty };
