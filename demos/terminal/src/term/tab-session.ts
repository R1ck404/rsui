import * as fs from 'fs';
import { gunzipSync, gzip } from 'zlib';
import type { Settings } from '../settings';
import { paneRects, stackedPanes } from '../strip/panes';
import type { PaneRect } from '../strip/panes';
import type { Pos, Selection, Session } from './Session';
import type { Column } from '../workspace';
import type { Terminal } from '../vt/terminal';
import { appendEvent, replayClock } from './timeline';
import type { Recorded } from './timeline';
import type { Crop } from './recording-edits';
import { recordingKey } from './recording-keys';
import type { KeystrokeAnimation, KeystrokeCategory, KeystrokeDuration, KeystrokeLayout, KeystrokeLimit, KeystrokePosition, RecordedKeystroke } from './recording-keys';
import type { KeyInfo } from '../vt/keys';

const styleKeys = ['background', 'backgroundColor', 'backgroundIntensity', 'backgroundSpeed', 'backgroundDensity', 'studio',
    'scheme', 'schemeOverrides', 'fontFamily', 'fontSize', 'lineHeight', 'ligatures', 'cursorShape', 'cursorBlink',
    'smoothCursor', 'cursorTrail', 'animations', 'graphics', 'showMarks', 'textPadding', 'radius',
    'columnOpacity', 'prompt'] as const;
export type RecordedStyle = Pick<Settings, typeof styleKeys[number]>;
export type RecordingEdits = {
    scheme?: string;
    fg?: number;
    bg?: number;
    cursor?: number;
    background?: Settings['background'];
    from?: number;
    to?: number;
    crop?: Crop;
    showKeystrokes?: boolean;
    keystrokeCategories?: KeystrokeCategory[];
    keystrokePosition?: KeystrokePosition;
    keystrokeDuration?: KeystrokeDuration;
    keystrokeLayout?: KeystrokeLayout;
    keystrokeLimit?: KeystrokeLimit;
    keystrokeAnimation?: KeystrokeAnimation;
};
type Timed<T> = { t: number; value: T };
export type RecordedView = { top: number | null; selection: Selection | null; cursor?: Pos; folds: number[]; bookmarks: number[]; title: string };
export interface PaneRecording {
    id: number;
    name: string;
    initial: ReturnType<Terminal['snapshot']>;
    events: Recorded[];
    views: Timed<RecordedView>[];
}
export interface TabRecording {
    format: 'rsui-terminal-tab';
    version: 1;
    duration: number;
    panes: PaneRecording[];
    layout: Timed<{ panes: { id: number; rect: PaneRect }[]; active: number | null; showHeader: boolean }>[];
    styles: Timed<RecordedStyle>[];
    keystrokes?: RecordedKeystroke[];
    edits?: RecordingEdits;
}

function addChange<T>(track: Timed<T>[], t: number, value: T) {
    const previous = track.at(-1);
    if (previous && JSON.stringify(previous.value) === JSON.stringify(value)) return;
    const entry = { t, value };
    if (previous?.t === t) track[track.length - 1] = entry;
    else track.push(entry);
}

export function stateAt<T>(track: Timed<T>[], t: number): T {
    let low = 0, high = track.length;
    while (low < high) {
        const middle = (low + high) >>> 1;
        if (track[middle].t <= t) low = middle + 1;
        else high = middle;
    }
    return track[Math.max(0, low - 1)].value;
}

export class TabSessionRecorder {
    #file: string;
    #elapsed = 0;
    #resumed: number | null = null;
    #style?: Settings;
    #listeners = new Map<number, () => void>();
    #recording: TabRecording = { format: 'rsui-terminal-tab', version: 1, duration: 0, panes: [], layout: [], styles: [] };

    constructor(file: string) { this.#file = file; }

    #time() {
        return Math.round(this.#elapsed + (this.#resumed === null ? 0 : replayClock() - this.#resumed));
    }

    update(tab: Column, settings: Settings, shown: boolean) {
        if (shown && this.#resumed === null) this.#resumed = replayClock();
        else if (!shown && this.#resumed !== null) {
            this.#elapsed += replayClock() - this.#resumed;
            this.#resumed = null;
        }
        const t = this.#time();
        if (this.#style !== settings) {
            this.#style = settings;
            const style = Object.fromEntries(styleKeys.map((key) => [key, settings[key]])) as RecordedStyle;
            addChange(this.#recording.styles, this.#recording.styles.length ? t : 0, style);
        }
        const active = tab.sessions[tab.active]?.id ?? null;
        const geometry = tab.zoomed && active !== null ? tab.active : tab.paneLayout ?? stackedPanes(tab.sessions.length);
        const panes = paneRects(geometry).map(({ index, rect }) => ({ id: tab.sessions[index].id, rect }));
        const showHeader = settings.layout === 'strip' || tab.sessions.length > 1;
        addChange(this.#recording.layout, this.#recording.layout.length ? t : 0, { panes, active, showHeader });
        for (const session of tab.sessions) {
            if (!this.#listeners.has(session.id)) this.#attach(session, t);
        }
        for (const [id, stop] of this.#listeners) {
            if (tab.sessions.some((session) => session.id === id)) continue;
            stop();
            this.#listeners.delete(id);
        }
    }

    recordKey(event: KeyInfo) {
        if (this.#resumed === null) return;
        const keys = recordingKey(event);
        if (keys) (this.#recording.keystrokes ??= []).push({ t: this.#time(), keys });
    }

    #attach(session: Session, t: number) {
        const pane: PaneRecording = {
            id: session.id, name: session.displayName,
            // Freeze lists that the live emulator mutates in place.
            initial: JSON.parse(JSON.stringify(session.term.snapshot())),
            events: [], views: [],
        };
        const view = (at: number) => addChange(pane.views, at, {
            top: session.scrolledTop, selection: session.selection, cursor: session.copyMode?.cursor,
            folds: [...session.folds], bookmarks: [...session.bookmarks.keys()], title: session.displayName,
        });
        this.#recording.panes.push(pane);
        view(t);
        const unsubscribe = session.subscribe(() => view(this.#time()));
        const pending: Omit<Recorded, 't'>[] = [];
        let metrics = { ...session.term.cellPx, graphics: session.term.graphics };
        let syncTimeout: ReturnType<typeof setTimeout> | null = null;
        const flush = () => {
            if (syncTimeout !== null) clearTimeout(syncTimeout);
            syncTimeout = null;
            // The live pane waits for synchronized updates, with a 150 ms timeout.
            const t = this.#time();
            for (const event of pending) appendEvent(pane.events, { ...event, t }, pane.events.at(-1)?.t === t);
            pending.length = 0;
        };
        const unobserve = session.term.observeRecording((kind, data) => {
            const { w, h } = session.term.cellPx;
            const { graphics } = session.term;
            const event: Omit<Recorded, 't'> = { kind, data };
            if (kind === 'o') event.wall = Date.now();
            if (metrics.w !== w || metrics.h !== h || metrics.graphics !== graphics) {
                metrics = { w, h, graphics };
                event.metrics = metrics;
            }
            pending.push(event);
            if (!session.term.synchronized) flush();
            else if (syncTimeout === null) syncTimeout = setTimeout(flush, 150);
        });
        this.#listeners.set(session.id, () => { unobserve(); unsubscribe(); flush(); });
    }

    async stop() {
        for (const stop of this.#listeners.values()) stop();
        this.#listeners.clear();
        this.#recording.duration = this.#time();
        await saveTabSession(this.#file, this.#recording);
        return this.#file;
    }
}

export async function saveTabSession(file: string, data: TabRecording) {
    const temporary = `${file}.${process.pid}-${Math.random().toString(36).slice(2)}.part`;
    const input = Buffer.from(JSON.stringify(data), 'utf8');
    const packed = await new Promise<Buffer>((resolve, reject) => gzip(input, { level: 6 }, (error, result) =>
        error ? reject(error) : resolve(result)));
    try {
        await fs.promises.writeFile(temporary, packed);
        await fs.promises.rename(temporary, file);
    } catch (error) {
        await fs.promises.unlink(temporary).catch(() => {});
        throw error;
    }
}

export function loadTabSession(file: string): TabRecording | null {
    let fd: number | undefined;
    try {
        fd = fs.openSync(file, 'r');
        const bytes = Buffer.alloc(2);
        if (fs.readSync(fd, bytes, 0, 2, 0) !== 2 || bytes[0] !== 31 || bytes[1] !== 139) return null;
        if (fs.statSync(file).size > 64 * 1024 * 1024) return null;
        const data = JSON.parse(gunzipSync(fs.readFileSync(file), { maxOutputLength: 192 * 1024 * 1024 }).toString('utf8')) as TabRecording;
        if (data?.format !== 'rsui-terminal-tab' || data.version !== 1 || !Number.isSafeInteger(data.duration) || data.duration < 0 ||
            !Array.isArray(data.layout) || !data.layout.length || !data.layout.every((entry) => Array.isArray(entry?.value?.panes)) ||
            !Array.isArray(data.styles) || !data.styles.length || !Array.isArray(data.panes) || !data.panes.length || data.panes.length > 100) return null;
        if (!data.panes.every((pane) => Number.isInteger(pane.initial?.cols) && pane.initial.cols > 0 && pane.initial.cols <= 1000 &&
            Number.isInteger(pane.initial.rows) && pane.initial.rows > 0 && pane.initial.rows <= 500 &&
            Array.isArray(pane.events) && pane.views?.length)) return null;
        if (data.keystrokes !== undefined && (!Array.isArray(data.keystrokes) || !data.keystrokes.every((event, i, events) =>
            Number.isSafeInteger(event?.t) && event.t >= (events[i - 1]?.t ?? 0) && event.t <= data.duration &&
            Array.isArray(event.keys) && event.keys.length >= 1 && event.keys.length <= 5 &&
            event.keys.every((key) => typeof key === 'string' && key.length > 0 && key.length <= 16 && !/[\x00-\x1f\x7f-\x9f]/.test(key))))) {
            delete data.keystrokes;
        }
        return data;
    } catch {
        return null;
    } finally {
        if (fd !== undefined) fs.closeSync(fd);
    }
}
