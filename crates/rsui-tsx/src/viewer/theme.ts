// The inspector's colors, sizes and limits, what it's told by an app or a trace, and a few small helpers.

export type Handled = { action: string; key: string; took: number; note: string | null; rejected: boolean };
export type Render = { name: string | null; cause: string; took: number; own: number; changed: boolean; depth: number; why: string | null };
export type Frame = { number: number; at: number; script: number; layout: number; paint: number; gpu: number; present?: number; handled: Handled[]; renders: Render[] };
export type Change = { number: number; component: string; said: string };
export type Trace = { app: string; file: string | null; frames: Frame[]; history: Change[]; summary: string };
export type Running = { pid: number; pipe: string; app: string; file: string; started: number; place: string };
export type Heard = { number: number; line: string };
export type Logpoint = { at: string; expression: string; hits: number };
export type Breakpoint = { at: string; hits: number };
export type Local = { name: string; value: string };
export type StoppedFrame = { at: string; function: string; debugged: boolean; locals: Local[] };
export type Stopped = { reason: string; at: string; function: string; source: { line: number; text: string }[]; frames: StoppedFrame[] };
export type Input = { number: number; at: number; kind: string; said: string };
export type El = { path: number[]; tag: string; key: string | null; name: string; props: Record<string, any>; actions: string[]; rect: number[] | null; children: El[]; more: number };
export type Made = { component: string | null; around: string[]; why: string | null; hooks: string[]; rect: number[] | null; inspecting: boolean };
export type Answer = { error?: string; said?: string; [more: string]: any };
export type Ask = (request: object) => Promise<Answer>;

export const BUDGET = 16.667;
/// The most frames the inspector keeps of an app it's looking at, as the app does.
export const MOST_FRAMES = 4000;
/// The most lines of what's been said it keeps, and how many of a long list are shown.
export const MOST_HEARD = 1000;
export const MOST_LISTED = 300;
/// How often it asks an app what's new, and what's in its window while that's shown.
export const ASK_EVERY = 250;
export const TREE_EVERY = 1000;
export const UI = 'Segoe UI';
export const MONO = 'Consolas';
export const ROW = 20;
/// How wide a pane has to be for what's in it, below which it scrolls across, as it does docked along an app's right.
export const PANEL_LEAST = 820;

/// The colors of a browser's inspector, dark and light.
export function colors(dark: boolean) {
    return dark
        ? {
              page: '#202124', bar: '#292a2d', raised: '#35363a', line: '#3c4043', strong: '#5f6368', text: '#e8eaed', dim: '#9aa0a6',
              hover: '#2f3134', selected: '#33445c', chosen: '#3c4043', accent: '#8ab4f8', onAccent: '#202124', field: '#202124',
              tag: '#5db0d7', attr: '#9bbbdc', value: '#f29766', str: '#f29766', num: '#9980ff', kw: '#c58af9', comment: '#9aa0a6', fn: '#78d9ec',
              error: '#f28b82', errorBack: '#4e3534', warn: '#fdd663', warnBack: '#413c26', ok: '#81c995',
              paused: '#fdd663', pausedBack: '#3d3a28', exec: '#2c4268', breakpoint: '#5186ec', logpoint: '#e37400',
              script: '#e3b341', layout: '#a487f2', paint: '#6fc26d', gpu: '#6a9fe6',
              margin: '#7b5a3a', border: '#8a7a3a', padding: '#5d6e3a', content: '#3a5e78', box: '#e8eaed',
          }
        : {
              page: '#ffffff', bar: '#f1f3f4', raised: '#ffffff', line: '#d6d9dc', strong: '#9aa0a6', text: '#1f1f1f', dim: '#5f6368',
              hover: '#f1f4f8', selected: '#cfe8fc', chosen: '#e8eaed', accent: '#1a73e8', onAccent: '#ffffff', field: '#ffffff',
              tag: '#881280', attr: '#994500', value: '#1a1aa6', str: '#c41a16', num: '#1c00cf', kw: '#aa0d91', comment: '#236e25', fn: '#0b6e99',
              error: '#d93025', errorBack: '#fce8e6', warn: '#9b5600', warnBack: '#fef7e0', ok: '#188038',
              paused: '#7a5a00', pausedBack: '#fef7e0', exec: '#dbe8fd', breakpoint: '#1a73e8', logpoint: '#e37400',
              script: '#f2c14e', layout: '#9b84e4', paint: '#62b36a', gpu: '#7aa6e8',
              margin: '#f9cc9d', border: '#fddd9b', padding: '#c3d08b', content: '#8cb6c0', box: '#1f1f1f',
          };
}
export type Theme = ReturnType<typeof colors>;
export const DARK = colors(true);
export const LIGHT = colors(false);

export const PARTS: [keyof Frame, string, keyof Theme][] = [
    ['script', 'Scripting', 'script'],
    ['layout', 'Layout', 'layout'],
    ['paint', 'Painting', 'paint'],
    ['gpu', 'GPU', 'gpu'],
];

export function ms(value: number): string {
    if (value > 0 && value < 0.1) {
        return '<0.1 ms';
    }
    return `${value.toFixed(1)} ms`;
}

export function total(frame: Frame): number {
    return frame.script + frame.layout + frame.paint + frame.gpu;
}

export function plural(count: number, what: string): string {
    return `${count} ${what}${count === 1 ? '' : 's'}`;
}
