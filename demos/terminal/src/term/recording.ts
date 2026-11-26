// Recordings, in asciinema's .cast format (version 2): a line of JSON about the recording, then a line for each thing the program
// printed, with when. Any player for the format can play them, and so can the one here.

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import type { Recorded } from './timeline';
import type { Terminal } from '../vt/terminal';

export interface CastHeader {
    version: 2;
    width: number;
    height: number;
    timestamp: number;
    title?: string;
    env?: Record<string, string>;
}

export interface Cast {
    header: CastHeader;
    events: Recorded[];
    /** Seconds the last event happened at. */
    duration: number;
    initial?: Terminal;
}

export function castText(header: CastHeader, events: Recorded[]): string {
    const lines = [JSON.stringify(header)];
    for (const event of events) lines.push(eventLine(event));
    return lines.join('\n') + '\n';
}

const eventLine = (event: Recorded) => JSON.stringify([Math.round(event.t) / 1000, event.kind ?? 'o', event.data]);

export function parseCast(text: string): Cast | null {
    const lines = text.matchAll(/[^\r\n]+/g);
    let first = lines.next();
    while (!first.done && !first.value[0].trim()) first = lines.next();
    if (first.done) return null;
    let header: CastHeader;
    try {
        header = JSON.parse(first.value[0]) as CastHeader;
    } catch {
        return null;
    }
    if (header?.version !== 2 || !Number.isInteger(header.width) || !Number.isInteger(header.height) ||
        header.width < 1 || header.width > 500 || header.height < 1 || header.height > 300) return null;
    const events: Recorded[] = [];
    for (const [line] of lines) {
        if (!line.trim()) continue;
        try {
            const item = JSON.parse(line);
            if (!Array.isArray(item) || typeof item[0] !== 'number' || !Number.isFinite(item[0]) || item[0] < 0 ||
                (item[1] !== 'o' && item[1] !== 'r') || typeof item[2] !== 'string') continue;
            const t = Math.max(events.at(-1)?.t ?? 0, item[0] * 1000);
            events.push({ t, data: item[2], kind: item[1] });
        } catch {
            // A line that isn't an event is skipped.
        }
    }
    return { header, events, duration: events.length > 0 ? events[events.length - 1].t / 1000 : 0 };
}

export function recordingsFolder(): string {
    return path.join(os.homedir(), 'terminal-recordings');
}

const two = (n: number) => String(n).padStart(2, '0');

function stamp(time: number) {
    const d = new Date(time);
    return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}_${two(d.getHours())}-${two(d.getMinutes())}-${two(d.getSeconds())}-${String(d.getMilliseconds()).padStart(3, '0')}`;
}

export async function tabRecordingPath(): Promise<string> {
    const folder = recordingsFolder();
    await fs.promises.mkdir(folder, { recursive: true });
    return path.join(folder, `${stamp(Date.now())}-${process.pid}.tabrec`);
}

export async function saveRecording(recording: { startedAt: number; events: Recorded[]; cols: number; rows: number }, title: string): Promise<string> {
    const folder = recordingsFolder();
    await fs.promises.mkdir(folder, { recursive: true });
    const name = `${stamp(recording.startedAt)}.cast`;
    const file = path.join(folder, name);
    const header: CastHeader = { version: 2, width: recording.cols, height: recording.rows, timestamp: Math.floor(recording.startedAt / 1000), title, env: { TERM: 'xterm-256color' } };
    const temporary = file + `.${process.pid}-${Math.random().toString(36).slice(2)}.tmp`;
    try {
        let lines = [JSON.stringify(header)];
        let size = lines[0].length;
        for (const event of recording.events) {
            const line = eventLine(event);
            lines.push(line);
            size += line.length;
            if (size < 256 * 1024) continue;
            await fs.promises.appendFile(temporary, lines.join('\n') + '\n');
            lines = [];
            size = 0;
        }
        if (lines.length) await fs.promises.appendFile(temporary, lines.join('\n') + '\n');
        await fs.promises.rename(temporary, file);
        return file;
    } catch (error) {
        await fs.promises.unlink(temporary).catch(() => {});
        throw error;
    }
}

export interface RecordingFile {
    file: string;
    name: string;
    size: number;
    modified: number;
}

export function listRecordings(): RecordingFile[] {
    const folder = recordingsFolder();
    try {
        return fs
            .readdirSync(folder)
            .filter((name) => name.endsWith('.tabrec') || name.endsWith('.cast') || (name.endsWith('.mp4') && !name.endsWith('.part.mp4')))
            .map((name) => {
                const file = path.join(folder, name);
                const stat = fs.statSync(file);
                return { file, name, size: stat.size, modified: Number(stat.mtimeMs) };
            })
            .sort((a, b) => b.modified - a.modified);
    } catch {
        return [];
    }
}

export async function loadRecording(file: string): Promise<Cast | null> {
    try {
        return parseCast(await fs.promises.readFile(file, 'utf8'));
    } catch {
        return null;
    }
}
