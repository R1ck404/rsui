// The prompt editor and the PowerShell integration share the same segment definitions and ANSI styles.

export const PROMPT_SEGMENTS = {
    shell: { label: 'Shell', sample: 'PS', fg: 0x145d63, bg: 0xe7f3ed },
    folder: { label: 'Folder', sample: '~\\work', fg: 0xffffff, bg: 0x007e80 },
    git: { label: 'Git branch', sample: 'git:main', fg: 0xe5e9ff, bg: 0x394b77 },
    status: { label: 'Exit code on failure', sample: 'exit 1', fg: 0xfff0f2, bg: 0x8c364b },
    time: { label: 'Time', sample: '09:41', fg: 0xe6f1f4, bg: 0x3f505e },
    user: { label: 'Username', sample: 'dev', fg: 0xf6ebff, bg: 0x694b80 },
};

export type SegmentId = keyof typeof PROMPT_SEGMENTS;
export const SEGMENT_IDS = Object.keys(PROMPT_SEGMENTS) as SegmentId[];
export type PromptStyle = 'shell' | 'minimal' | 'powerline';
export type FolderMode = 'name' | 'home' | 'full';
export type PromptEdge = 'arrow' | 'round' | 'flat';

export interface PromptSettings {
    style: PromptStyle;
    /** Enabled segments in the order they appear in the shell. */
    order: SegmentId[];
    /** Positions of all rows in the editor, including disabled segments. */
    segmentOrder?: SegmentId[];
    colors: Partial<Record<SegmentId, { fg?: number; bg?: number }>>;
    folder: FolderMode;
    edge: PromptEdge;
    newLine: boolean;
}

export const DEFAULT_PROMPT: PromptSettings = {
    style: 'shell',
    order: ['shell', 'folder'],
    segmentOrder: SEGMENT_IDS,
    colors: {},
    folder: 'home',
    edge: 'arrow',
    newLine: false,
};

export function promptSegmentOrder({ order, segmentOrder }: Pick<PromptSettings, 'order' | 'segmentOrder'>): SegmentId[] {
    const saved = segmentOrder ?? [];
    // An older or partial settings import can supply just the enabled order.
    if (saved.filter((id) => order.includes(id)).join(',') !== order.join(',')) {
        return [...order, ...SEGMENT_IDS.filter((id) => !order.includes(id))];
    }
    return [...saved, ...SEGMENT_IDS.filter((id) => !saved.includes(id))];
}

const RESET = '\x1b[0m';
const DEFAULT_BG = '\x1b[49m';
const EDGES = { arrow: '\ue0b0', round: '\ue0b4', flat: '' };
const paint = (color: number, layer: 38 | 48) => `\x1b[${layer};2;${color >> 16 & 255};${color >> 8 & 255};${color & 255}m`;

export function compilePrompt(config: PromptSettings) {
    return {
        style: config.style,
        folder: config.folder,
        edge: EDGES[config.edge],
        suffix: config.newLine ? '\r\n> ' : config.style === 'minimal' ? ' > ' : ' ',
        segments: config.order.map((id) => {
            const colors = config.colors[id];
            const bg = colors?.bg ?? PROMPT_SEGMENTS[id].bg;
            return {
                id,
                fg: config.style === 'minimal' && colors?.fg === undefined ? '\x1b[39m' : paint(colors?.fg ?? PROMPT_SEGMENTS[id].fg, 38),
                bg: paint(bg, 48),
                edgeColor: paint(bg, 38),
            };
        }),
    };
}

export function previewPrompt(config: PromptSettings): string {
    if (config.style === 'shell') return 'PS C:\\Users\\dev\\work> ';
    const { style, edge, suffix, segments } = compilePrompt(config);
    let text = '';
    let previous = '';
    for (const segment of segments) {
        const label = segment.id === 'folder'
            ? { name: 'work', home: '~\\work', full: 'C:\\Users\\dev\\work' }[config.folder]
            : PROMPT_SEGMENTS[segment.id].sample;
        if (style === 'powerline') {
            if (previous && edge) text += segment.bg + previous + edge;
            text += segment.bg + segment.fg + ' ' + label + ' ';
            previous = segment.edgeColor;
        } else {
            if (text) text += ' ';
            text += segment.fg + label + RESET;
        }
    }
    if (previous && edge) text += DEFAULT_BG + previous + edge;
    return text + RESET + (text ? suffix : '> ');
}
