// The inspector's link to an app it looks at while that runs, over the pipe the app listens on, and which apps there are.
import { readFileSync, readdirSync } from 'fs';
import { connect } from 'net';
import type { Answer, Running } from './theme';

/// One line of JSON each way with an app, over its pipe, each answer coming in the order it was asked.
export class Link {
    #socket: any;
    #waiting: ((answer: Answer) => void)[] = [];
    #given = '';
    gone = false;

    constructor(pipe: string, onGone: (why: string) => void) {
        this.#socket = connect(pipe);
        this.#socket.setEncoding?.('utf8');
        this.#socket.on('data', (chunk: unknown) => {
            this.#given += String(chunk);
            let end;
            while ((end = this.#given.indexOf('\n')) >= 0) {
                const line = this.#given.slice(0, end);
                this.#given = this.#given.slice(end + 1);
                let answer: Answer;
                try {
                    answer = JSON.parse(line);
                } catch {
                    answer = { error: `the app said something that isn't JSON: ${line.slice(0, 80)}` };
                }
                this.#waiting.shift()?.(answer);
            }
        });
        let why = 'The app has stopped.';
        this.#socket.on('error', (error: { code?: string; message?: string }) => {
            why = error.code === 'ENOENT' ? 'The app is no longer running.' : `The app can't be reached: ${error.message ?? error}`;
        });
        this.#socket.on('close', () => {
            this.gone = true;
            for (const waiting of this.#waiting.splice(0)) {
                waiting({ error: why });
            }
            onGone(why);
        });
    }

    ask(request: object): Promise<Answer> {
        if (this.gone) {
            return Promise.resolve({ error: 'The app has stopped.' });
        }
        return new Promise((done) => {
            this.#waiting.push(done);
            this.#socket.write(JSON.stringify(request) + '\n');
        });
    }

    close() {
        this.gone = true;
        this.#socket.destroy();
    }
}

/// The apps that say they can be looked at, newest first.
export function runningApps(): Running[] {
    const folder = process.env.RSUI_LIVE;
    if (!folder) {
        return [];
    }
    let names: string[] = [];
    try {
        names = readdirSync(folder);
    } catch {
        return [];
    }
    const found: Running[] = [];
    for (const name of names) {
        if (!name.endsWith('.json')) {
            continue;
        }
        const place = `${folder}\\${name}`;
        try {
            found.push({ ...JSON.parse(readFileSync(place, 'utf8')), place });
        } catch {
            // It's being written, or it's not one of these.
        }
    }
    const wanted = Number(process.env.RSUI_LIVE_PID);
    return found.filter((one) => !wanted || one.pid === wanted).sort((a, b) => b.started - a.started);
}

// Pieces everything's made of.
