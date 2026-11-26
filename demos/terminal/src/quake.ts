// The dropdown terminal, as Quake had its console: a key that works from anywhere slides the window down from the top of the screen
// the pointer is on, and the same key slides it away. It's the same window, so every column is as it was.

import { displays, mainWindow } from 'rsui/desktop';
import type { Display } from 'rsui/desktop';

export type QuakeState = 'off' | 'shown' | 'hidden';

interface Saved {
    x: number;
    y: number;
    width: number;
    height: number;
}

const easeOut = (t: number) => 1 - Math.pow(1 - t, 3);

export class Quake {
    state: QuakeState = 'off';
    #saved: Saved | null = null;
    #busy = false;
    #height: () => number;
    #changed: (state: QuakeState) => void;

    constructor(height: () => number, changed: (state: QuakeState) => void) {
        this.#height = height;
        this.#changed = changed;
    }

    #set(state: QuakeState) {
        this.state = state;
        this.#changed(state);
    }

    /** The room it takes at the top of a screen, in the units the window is moved in. */
    #room(display: Display) {
        const scale = display.scale || 1;
        const width = Math.round(display.workWidth / scale);
        const height = Math.round((display.workHeight / scale) * this.#height());
        return { x: Math.round(display.workX / scale), y: Math.round(display.workY / scale), width, height };
    }

    #screen(): Display | null {
        try {
            const [px, py] = displays.pointer();
            return displays.forPoint(px, py) ?? displays.primary();
        } catch {
            return displays.primary();
        }
    }

    async #slide(fromY: number, toY: number, x: number, ms: number) {
        const start = Date.now();
        await new Promise<void>((done) => {
            const step = () => {
                const t = Math.min(1, (Date.now() - start) / ms);
                mainWindow.moveTo(x, Math.round(fromY + (toY - fromY) * easeOut(t)));
                if (t >= 1) done();
                else setTimeout(step, 8);
            };
            step();
        });
    }

    /** The key: show it, hide it, or take the window up to the top the first time. */
    async toggle(animate = true) {
        if (this.#busy) return;
        this.#busy = true;
        try {
            if (this.state === 'shown') {
                const room = this.#saved ? mainWindow.getState() : null;
                if (room && animate) await this.#slide(room.y, room.y - room.height - 8, room.x, 180);
                mainWindow.hide();
                this.#set('hidden');
                return;
            }
            const display = this.#screen();
            if (!display) return;
            if (this.state === 'off') {
                const now = mainWindow.getState();
                this.#saved = { x: now.x, y: now.y, width: now.width, height: now.height };
            }
            const room = this.#room(display);
            mainWindow.setAlwaysOnTop(true);
            mainWindow.resize(room.width, room.height);
            mainWindow.moveTo(room.x, room.y - room.height - 8);
            mainWindow.show();
            if (animate) await this.#slide(room.y - room.height - 8, room.y, room.x, 220);
            else mainWindow.moveTo(room.x, room.y);
            mainWindow.focus();
            this.#set('shown');
        } finally {
            this.#busy = false;
        }
    }

    /** Back to being an ordinary window, where it was. */
    leave() {
        if (this.state === 'off') return;
        mainWindow.setAlwaysOnTop(false);
        const saved = this.#saved;
        if (saved) {
            mainWindow.resize(saved.width, saved.height);
            mainWindow.moveTo(saved.x, saved.y);
        }
        mainWindow.show();
        this.#set('off');
    }
}
