// What an app says about where its time went, as timed spans (see `rsui-spans`), kept here as the viewer gets them: each frame,
// and inside it what came in, the script that ran, each component that rendered, layout, paint and the GPU, and, while it's
// recording, where JavaScript was every millisecond. Nothing here draws: `timeline.tsx` does, from what this works out.

/// Nanoseconds in a millisecond, as spans are timed in nanoseconds.
export const NS = 1_000_000;
/// How much is kept here, which is as much as the app keeps.
export const KEEP_NS = 30_000 * NS;

export type Kind = { name: string; category: string; about: string };
export type Span = { kind: number; thread: number; start: number; end: number; label: number; arg: number };
export type Sample = { at: number; stack: number };
/// A span placed for drawing: how deep it is, with what it's inside counted, and how long it took itself.
export type Placed = Span & { depth: number; self: number };

/// What `news` says about spans, as `live.rs` writes it.
export type SpansNews = {
    next: number;
    recs: number[];
    labels: string[];
    stacks: number[][];
    now: number;
    threads: string[];
    level: number;
    recording: boolean;
    dropped: number;
    kinds?: [string, string, string][];
};

export class Spans {
    kinds: Kind[] = [];
    labels: string[] = [];
    stacks: number[][] = [];
    threads: string[] = [];
    /// Every span but samples, in the order they ended, which is the order they're kept in.
    spans: Span[] = [];
    samples: Sample[] = [];
    /// The frames, oldest first.
    frames: Span[] = [];
    next = 0;
    now = 0;
    level = 0;
    recording = false;
    /// Goes up each time something's added, for what's worked out from this to know when to work it out again.
    version = 0;
    #frameKind = -1;
    #sampleKind = -1;

    /// What to ask for next, in `news`.
    asking() {
        return { spans: this.next, labels: this.labels.length, stacks: this.stacks.length };
    }

    /// Takes in what `news` said. Says whether anything changed.
    absorb(news: SpansNews | undefined): boolean {
        if (!news) {
            return false;
        }
        if (news.kinds) {
            this.kinds = news.kinds.map(([name, category, about]) => ({ name, category, about }));
            this.#frameKind = this.kinds.findIndex((kind) => kind.name === 'Frame');
            this.#sampleKind = this.kinds.findIndex((kind) => kind.name === 'Sample');
        }
        this.labels.push(...news.labels);
        this.stacks.push(...news.stacks);
        this.threads = news.threads;
        const changed = news.recs.length > 0 || this.recording !== news.recording || this.level !== news.level;
        this.now = news.now;
        this.level = news.level;
        this.recording = news.recording;
        this.next = news.next;
        const recs = news.recs;
        for (let at = 0; at + 5 < recs.length; at += 6) {
            const span: Span = { kind: recs[at], thread: recs[at + 1], start: recs[at + 2], end: recs[at + 2] + recs[at + 3], label: recs[at + 4], arg: recs[at + 5] };
            if (span.kind === this.#sampleKind) {
                this.samples.push({ at: span.start, stack: span.label });
            } else {
                this.spans.push(span);
                if (span.kind === this.#frameKind) {
                    this.frames.push(span);
                }
            }
        }
        if (changed) {
            this.#forgetOld();
            this.version += 1;
        }
        return changed;
    }

    #forgetOld() {
        const oldest = this.now - KEEP_NS;
        const drop = <T>(list: T[], end: (item: T) => number) => {
            let gone = 0;
            while (gone < list.length && end(list[gone]) < oldest) {
                gone += 1;
            }
            if (gone > 0) {
                list.splice(0, gone);
            }
        };
        // Spans are kept in the order they ended, so the oldest are first.
        drop(this.spans, (span) => span.end);
        drop(this.frames, (span) => span.end);
        drop(this.samples, (sample) => sample.at);
    }

    kind(span: Span): Kind {
        return this.kinds[span.kind] ?? { name: '?', category: 'other', about: '' };
    }

    /// What a span's called, as a block in the chart: its kind, and what it was about.
    title(span: Span): string {
        const kind = this.kind(span);
        const label = this.labels[span.label] ?? '';
        switch (kind.name) {
            case 'Render':
                return label || 'a component';
            case 'Frame':
                return `Frame ${span.arg}`;
            case 'GC':
                return span.arg ? 'Major GC' : 'Minor GC';
            case 'Layout':
                return span.arg ? `Layout (${span.arg} boxes)` : 'Layout';
            case 'Shape text':
                return `Shape text (${span.arg} chars)`;
            default:
                return label ? `${kind.name}: ${label}` : kind.name;
        }
    }

    /// The oldest time there's anything for, and the newest.
    extent(): [number, number] {
        const first = Math.min(this.spans[0]?.start ?? this.now, this.samples[0]?.at ?? this.now);
        return [Math.max(first, this.now - KEEP_NS), this.now];
    }

    /// The spans on `thread` that are at all between `from` and `to`, each with how deep it is and its own time, outermost first.
    placed(from: number, to: number, thread = 0): Placed[] {
        const inside = this.spans.filter((span) => span.thread === thread && span.end >= from && span.start <= to && span.kind !== this.#frameKind);
        // What started first comes first, and of two that started together, the longer, which the other's inside.
        inside.sort((a, b) => a.start - b.start || b.end - a.end);
        const open: Placed[] = [];
        const placed: Placed[] = [];
        for (const span of inside) {
            while (open.length > 0 && open[open.length - 1].end <= span.start) {
                open.pop();
            }
            // A span that doesn't fit inside the one before isn't inside it, though it started in it: it's beside it.
            while (open.length > 0 && open[open.length - 1].end < span.end) {
                open.pop();
            }
            const one: Placed = { ...span, depth: open.length, self: span.end - span.start };
            if (open.length > 0) {
                open[open.length - 1].self -= one.self;
            }
            placed.push(one);
            open.push(one);
        }
        return placed;
    }

    /// The samples between `from` and `to`.
    sampled(from: number, to: number): Sample[] {
        return this.samples.filter((sample) => sample.at >= from && sample.at <= to);
    }

    /// The frames that are at all between `from` and `to`.
    framesIn(from: number, to: number): Span[] {
        return this.frames.filter((frame) => frame.end >= from && frame.start <= to);
    }

    /// What a sampled function's label says: its name, and where it is as `file:line`.
    where(label: number): { name: string; at: string; file: string; line: number } {
        const text = this.labels[label] ?? '';
        const [name, at = ''] = text.split('\u0001');
        const colon = at.lastIndexOf(':');
        const file = colon > 0 ? at.slice(0, colon) : at;
        const line = colon > 0 ? Number(at.slice(colon + 1)) : 0;
        return { name, at, file, line };
    }
}

/// A row of what's been worked out over a range: what it is, the time it took itself and with what it called, and how many.
export type Row = { key: string; name: string; at: string; category: string; self: number; total: number; count: number };

/// Where time went between `from` and `to`, heaviest first: each kind of span by what it was about, and each function
/// JavaScript was sampled in, by its own time.
export function bottomUp(spans: Spans, from: number, to: number): Row[] {
    const rows = new Map<string, Row>();
    const add = (key: string, name: string, at: string, category: string, self: number, total: number) => {
        const row = rows.get(key);
        if (row) {
            row.self += self;
            row.total += total;
            row.count += 1;
        } else {
            rows.set(key, { key, name, at, category, self, total, count: 1 });
        }
    };
    const placed = spans.placed(from, to);
    for (const span of placed) {
        const kind = spans.kind(span);
        // What's clipped to the range counts as much as is inside it.
        const total = Math.min(span.end, to) - Math.max(span.start, from);
        const self = Math.max(0, Math.min(span.self, total));
        const label = spans.labels[span.label] ?? '';
        const name = kind.name === 'Render' ? label || 'a component' : kind.name === 'Script' || kind.name === 'Input' ? kind.name : spans.title(span);
        add(`span:${kind.name}:${name}`, name, kind.name === 'Render' ? 'Render' : '', kind.category, self, total);
    }
    // Each sample is a millisecond, or the time to the next if that's less.
    const samples = spans.sampled(from, to);
    for (let index = 0; index < samples.length; index += 1) {
        const sample = samples[index];
        const took = Math.min(NS, (samples[index + 1]?.at ?? sample.at + NS) - sample.at);
        const stack = spans.stacks[sample.stack] ?? [];
        const seen = new Set<number>();
        stack.forEach((label, depth) => {
            const where = spans.where(label);
            const inner = depth === stack.length - 1;
            // A function that's in the stack twice, as when it calls itself, counts once for its total.
            const key = `js:${where.name}:${where.file}`;
            const first = !seen.has(label);
            seen.add(label);
            const row = rows.get(key);
            if (row) {
                row.self += inner ? took : 0;
                row.total += first ? took : 0;
                row.count += inner ? 1 : 0;
            } else {
                rows.set(key, { key, name: where.name, at: where.at, category: 'js', self: inner ? took : 0, total: took, count: inner ? 1 : 0 });
            }
        });
    }
    return [...rows.values()].sort((a, b) => b.self - a.self || b.total - a.total);
}

/// How long each component took itself between `from` and `to`, and how often it rendered for nothing, for the treemap.
export function byComponent(spans: Spans, from: number, to: number): { name: string; self: number; renders: number; wasted: number }[] {
    const rows = new Map<string, { name: string; self: number; renders: number; wasted: number }>();
    for (const span of spans.placed(from, to)) {
        if (spans.kind(span).name !== 'Render') {
            continue;
        }
        const name = spans.labels[span.label] || 'a component';
        const row = rows.get(name) ?? { name, self: 0, renders: 0, wasted: 0 };
        row.self += span.self;
        row.renders += 1;
        // The render's number says whether it changed anything, in its third bit.
        row.wasted += span.arg & 4 ? 1 : 0;
        rows.set(name, row);
    }
    return [...rows.values()].sort((a, b) => b.self - a.self);
}

/// What JavaScript was sampled doing between `from` and `to`, as blocks: for each depth, a run of samples in the same function,
/// called from the same place, is one block.
export function sampleBlocks(spans: Spans, from: number, to: number): { depth: number; start: number; end: number; label: number }[] {
    const samples = spans.sampled(from, to);
    const blocks: { depth: number; start: number; end: number; label: number }[] = [];
    // What's open at each depth: its label, and the block it's in.
    let open: { depth: number; start: number; end: number; label: number }[] = [];
    for (let index = 0; index < samples.length; index += 1) {
        const sample = samples[index];
        const end = Math.min(sample.at + NS, samples[index + 1]?.at ?? sample.at + NS);
        const stack = spans.stacks[sample.stack] ?? [];
        // A gap of more than a couple of samples is time JavaScript wasn't running.
        const continues = open.length > 0 && sample.at - open[0].end < 2 * NS;
        let same = 0;
        if (continues) {
            while (same < open.length && same < stack.length && open[same].label === stack[same]) {
                same += 1;
            }
        }
        open = open.slice(0, same);
        for (const block of open) {
            block.end = end;
        }
        for (let depth = same; depth < stack.length; depth += 1) {
            const block = { depth, start: sample.at, end, label: stack[depth] };
            blocks.push(block);
            open.push(block);
        }
    }
    return blocks;
}

/// A frame's own time, from when it started to when it was handed to the GPU, without waiting for the screen.
export function frameWork(spans: Spans, frame: Span): number {
    return frame.end - frame.start;
}

export function formatNs(ns: number): string {
    const ms = ns / NS;
    if (ms >= 1000) {
        return `${(ms / 1000).toFixed(2)} s`;
    }
    if (ms >= 10) {
        return `${ms.toFixed(1)} ms`;
    }
    if (ms >= 0.1) {
        return `${ms.toFixed(2)} ms`;
    }
    if (ns <= 0) {
        return '0';
    }
    return `${Math.round(ns / 1000)} µs`;
}
