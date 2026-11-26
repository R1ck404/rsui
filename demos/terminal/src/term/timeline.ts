import { Terminal } from '../vt/terminal';
import type { Cast } from './recording';

export interface Recorded {
    /** Milliseconds since capture started. */
    t: number;
    data: string;
    kind?: 'o' | 'r';
    wall?: number;
    metrics?: { w: number; h: number; graphics: Terminal['graphics'] };
}

export const replayClock = () => performance.now();

export function applyRecording(term: Terminal, event: Recorded) {
    if (event.metrics) {
        const { w, h, graphics } = event.metrics;
        term.cellPx = { w, h };
        term.graphics = graphics;
    }
    if (event.kind !== 'r') {
        term.write(event.data, event.wall);
        return;
    }
    const match = /^(\d+)x(\d+)$/.exec(event.data);
    if (match && Number(match[1]) > 0 && Number(match[1]) <= 1000 && Number(match[2]) > 0 && Number(match[2]) <= 500) {
        term.resize(Number(match[1]), Number(match[2]));
    }
}

export function appendEvent(events: Recorded[], event: Recorded, coalesce: boolean) {
    const last = events[events.length - 1];
    if (coalesce && last && last.kind !== 'r' && event.kind !== 'r' && last.wall === event.wall && !event.metrics &&
        event.t - last.t < 16 && last.data.length + event.data.length < 65536) {
        last.data += event.data;
        return;
    }
    events.push(event);
}

interface Segment {
    start: number;
    initial: Terminal;
    events: Recorded[];
    chars: number;
}

/** Recent output in batches, each with the state immediately before it. */
export class RecentTimeline {
    readonly startedAt = replayClock();
    #segments: Segment[] = [];
    #chars = 0;

    add(live: Terminal, data: string, seconds: number, kind: Recorded['kind'] = 'o') {
        const t = replayClock() - this.startedAt;
        let segment = this.#segments.at(-1);
        if (!segment || segment.chars >= 256 * 1024 || segment.events.length >= 256 || t - segment.start >= 10000) {
            segment = { start: t, initial: live.fork(32), events: [], chars: 0 };
            this.#segments.push(segment);
        }
        segment.chars += data.length;
        this.#chars += data.length;
        appendEvent(segment.events, { t, data, kind }, true);
        const maxSegments = Math.max(2, Math.min(32, Math.floor(900000 / (live.cols * (2 * live.rows + 32)))));
        while (this.#segments.length > 1 && (this.#chars > 2 * 1024 * 1024 || this.#segments.length > maxSegments ||
            this.#segments[0].events.at(-1)!.t < t - seconds * 1000)) {
            this.#chars -= this.#segments.shift()!.chars;
        }
    }

    cast(title: string, seconds: number): Cast | null {
        if (!this.#segments.length) return null;
        const duration = replayClock() - this.startedAt;
        const start = Math.max(this.#segments[0].start, duration - seconds * 1000);
        let index = this.#segments.findIndex((segment) => segment.events.at(-1)!.t >= start);
        if (index < 0) index = this.#segments.length - 1;
        const initial = this.#segments[index].initial.fork();
        const events: Recorded[] = [];
        for (let i = index; i < this.#segments.length; i++) {
            for (const event of this.#segments[i].events) {
                if (event.t < start) applyRecording(initial, event);
                else events.push({ ...event, t: event.t - start });
            }
        }
        return {
            header: { version: 2, width: initial.cols, height: initial.rows, title, timestamp: Date.now() / 1000 },
            initial, events, duration: (duration - start) / 1000,
        };
    }
}

export class Playback {
    #checkpoints: { index: number; term: Terminal }[] = [];
    #applied = 0;
    #stride: number;
    #initial: Terminal;
    #events: Recorded[];
    term: Terminal;

    constructor(cast: Cast, configure?: (term: Terminal) => void) {
        this.#events = cast.events;
        this.#initial = cast.initial ?? new Terminal(cast.header.width, cast.header.height, 64);
        if (configure) {
            if (cast.initial) this.#initial = cast.initial.fork();
            configure(this.#initial);
        }
        this.term = this.#initial.fork();
        // Bound checkpoint count for both long recordings and large terminal grids.
        const cells = Math.max(1, this.term.cols * (this.term.rows * 2 + Math.max(64, this.term.scrollbackLength)));
        const count = Math.max(1, Math.min(24, Math.floor(600000 / cells)));
        this.#stride = Math.max(128, Math.ceil(cast.events.length / count));
    }

    seek(ms: number): boolean {
        const events = this.#events;
        let low = 0, high = events.length;
        while (low < high) {
            const mid = (low + high) >>> 1;
            if (events[mid].t <= ms) low = mid + 1;
            else high = mid;
        }
        if (this.#applied === low) return false;
        if (this.#applied > low) {
            let index = this.#checkpoints.length - 1;
            while (index >= 0 && this.#checkpoints[index].index > low) index--;
            const checkpoint = this.#checkpoints[index];
            this.term = (checkpoint?.term ?? this.#initial).fork();
            this.#applied = checkpoint?.index ?? 0;
        }
        while (this.#applied < low) {
            const start = this.#applied;
            const event = events[start];
            if (hasMetadata(event)) {
                applyRecording(this.term, event);
                this.#applied++;
            } else {
                const parts = [];
                let length = 0;
                do {
                    const data = events[this.#applied++].data;
                    parts.push(data);
                    length += data.length;
                } while (this.#applied < low && this.#applied % this.#stride !== 0 && !hasMetadata(events[this.#applied]) && length < 65536);
                this.term.write(parts.length === 1 ? parts[0] : parts.join(''));
            }
            if (this.#applied % this.#stride === 0 && this.#applied > (this.#checkpoints.at(-1)?.index ?? 0)) {
                this.#checkpoints.push({ index: this.#applied, term: this.term.fork() });
            }
        }
        return true;
    }
}

const hasMetadata = (event: Recorded) => event.kind === 'r' || event.wall !== undefined || event.metrics !== undefined;
