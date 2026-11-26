// The studio: make a background of your own. Choose a picture, a video or one of the drawn backgrounds (stars, light pillar, CRT...),
// pick what's done to it (as it is, ASCII, braille, half blocks, dither, halftone and more), tune it, and see it live behind some
// terminal text before putting it on.
//
// A file is read once (see media.ts), and the effect is drawn from a small grid sampled out of it, so changing any setting only
// draws again: nothing is decoded twice.

import { useEffect, useRef, useState } from 'react';
import { dialogs } from 'rsui/desktop';
import { Icon } from '../ui/Icon';
import { Field, Segmented, Slider, Switch } from '../ui/controls';
import type { Settings } from '../settings';
import { css, mix, withAlpha, tokens } from '../themes';
import type { Scheme } from '../themes';
import { DEFAULT_DESIGN, EFFECTS } from './effects';
import type { Charset, Design, EffectId, Fade } from './effects';
import { IMAGE_TYPES, VIDEO_TYPES, kindOf, load } from './media';
import type { Media } from './media';
import { BACKGROUNDS } from '../backgrounds/scenes';
import type { BackgroundKind, SceneParams } from '../backgrounds/scenes';
import { DEFAULT_SCENE, drawDesign, mediaSource, sceneSource } from '../backgrounds/source';
import type { FrameSource } from '../backgrounds/source';

/** The drawn backgrounds a design can start from. */
const SCENES = BACKGROUNDS.filter((b) => b.id !== 'none' && b.id !== 'custom');

const SWATCHES = [0xe4e4e7, 0x82aaff, 0xc3a6ff, 0x89ddff, 0x9fd39b, 0xe6c07b, 0xf07178, 0xff9e64, 0x1b2a4a, 0x2d1b4a, 0x0f2a25, 0x3a1f12];

const SAMPLE = ['~/code/web ❯ npm run dev', '', '  VITE v6.0.1  ready in 312 ms', '', '  ➜  Local:   http://localhost:5173/', '', '~/code/web ❯ git status', 'On branch main', 'nothing to commit, working tree clean', '~/code/web ❯ '];

export function Studio(props: { settings: Settings; scheme: Scheme; change: (patch: Partial<Settings>) => void; onApplied: () => void }) {
    const { settings, scheme } = props;
    const t = tokens(scheme);
    const [file, setFile] = useState<string | null>(settings.studio?.file || null);
    const [source, setSource] = useState<BackgroundKind | 'file'>(settings.studio?.source ?? 'file');
    const [scene, setScene] = useState<SceneParams>({ ...DEFAULT_SCENE, ...(settings.studio?.scene ?? {}) });
    const drawn = source !== 'file';
    const [design, setDesign] = useState<Design>({ ...DEFAULT_DESIGN, ...(settings.studio?.design ?? {}) });
    const [media, setMedia] = useState<Media | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [loading, setLoading] = useState(false);
    const [showText, setShowText] = useState(true);
    const set = (patch: Partial<Design>) => setDesign((was) => ({ ...was, ...patch }));

    useEffect(() => {
        if (!file) return;
        let live = true;
        setLoading(true);
        setError(null);
        load(file)
            .then((m) => live && setMedia(m))
            .catch((e: Error) => live && setError(e.message))
            .finally(() => live && setLoading(false));
        return () => {
            live = false;
        };
    }, [file]);

    const choose = async () => {
        const picked = await dialogs.open({
            title: 'A picture or a video',
            filters: [
                { name: 'Pictures and videos', extensions: [...IMAGE_TYPES, ...VIDEO_TYPES] },
                { name: 'Pictures', extensions: IMAGE_TYPES },
                { name: 'Videos', extensions: VIDEO_TYPES },
            ],
        });
        if (picked && kindOf(picked)) {
            setFile(picked);
            setSource('file');
        }
    };

    const ready = drawn || media !== null;
    const apply = () => {
        if (!ready) return;
        props.change({ studio: { file: file ?? '', design, source, scene }, background: 'custom' });
        props.onApplied();
    };
    // What the preview draws from, and what says it's another one.
    const make = (): FrameSource | null => (drawn ? sceneSource(source, scene) : media ? mediaSource(media, design.speed) : null);
    const sourceKey = drawn ? source + JSON.stringify(scene) : (media?.name ?? '') + ':' + design.speed;

    return (
        <div className="flex size-full flex-col" style={{ color: t.text }}>
            <div className="flex min-h-0 flex-1">
                <div className="relative min-w-0 flex-1 overflow-hidden">
                    {ready ? (
                        <Preview make={make} sourceKey={sourceKey} design={design} scheme={scheme} />
                    ) : (
                        <button className="absolute inset-0 flex size-full flex-col items-center justify-center gap-3" onClick={choose}>
                            <div className="flex size-12 items-center justify-center rounded-2xl" style={{ background: withAlpha(scheme.fg, 0.07), color: t.sub }}>
                                <Icon name="image" size={22} />
                            </div>
                            <div className="text-[14px] font-medium">{loading ? 'Reading it…' : 'Choose a picture or a video'}</div>
                            <div className="text-[12px]" style={{ color: t.faint }}>
                                {error ?? 'PNG, JPEG, GIF, WebP, MP4, MOV, WebM…'}
                            </div>
                        </button>
                    )}
                    {ready && showText ? (
                        <div className="pointer-events-none absolute left-5 top-4 text-[13px] leading-[21px]" style={{ fontFamily: `"${settings.fontFamily}", Consolas, monospace`, color: css(scheme.fg) }}>
                            {SAMPLE.map((line, i) => (
                                <div key={i} className="whitespace-pre">
                                    {line || ' '}
                                </div>
                            ))}
                        </div>
                    ) : null}
                    {media && !drawn ? (
                        <div className="absolute bottom-3 left-3 flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-[12px]" style={{ background: withAlpha(scheme.frame, 0.85), color: t.sub }}>
                            <Icon name={media.kind === 'video' ? 'video' : 'image'} size={13} />
                            <span className="max-w-[260px] truncate">{media.name}</span>
                            {media.kind === 'video' ? <span style={{ color: t.faint }}>{media.count} frames</span> : null}
                            <button className="ml-1 rounded px-1.5 hover:bg-white/10" style={{ color: t.text }} onClick={choose}>
                                Change
                            </button>
                        </div>
                    ) : null}
                </div>
                <div className="flex w-[300px] shrink-0 flex-col overflow-hidden" style={{ borderLeft: `1px solid ${t.border}` }}>
                    <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4 pt-3">
                        <Label scheme={scheme}>Start from</Label>
                        <button
                            className="mb-1 flex h-8 w-full items-center gap-2 rounded-lg px-2.5 text-left text-[12px] transition-colors duration-150"
                            style={{ background: !drawn ? withAlpha(scheme.fg, 0.14) : withAlpha(scheme.fg, 0.04), color: !drawn ? t.text : t.sub, fontWeight: !drawn ? 500 : 400 }}
                            onClick={() => (media ? setSource('file') : choose())}
                        >
                            <Icon name={media?.kind === 'video' ? 'video' : 'image'} size={13} />
                            <span className="min-w-0 flex-1 truncate">{media ? media.name : 'A picture or video…'}</span>
                        </button>
                        <div className="mb-3 grid grid-cols-3 gap-1">
                            {SCENES.map((b) => {
                                const on = b.id === source;
                                return (
                                    <button
                                        key={b.id}
                                        title={b.hint}
                                        className="h-8 truncate rounded-lg px-1 text-[12px] transition-colors duration-150"
                                        style={{ background: on ? withAlpha(scheme.fg, 0.14) : withAlpha(scheme.fg, 0.04), color: on ? t.text : t.sub, fontWeight: on ? 500 : 400 }}
                                        onClick={() => setSource(b.id)}
                                    >
                                        {b.name}
                                    </button>
                                );
                            })}
                        </div>
                        {drawn ? <SceneControls scene={scene} set={(patch) => setScene((was) => ({ ...was, ...patch }))} scheme={scheme} /> : null}
                        <div className="h-2" />
                        <Label scheme={scheme}>Effect</Label>
                        <div className="mb-5 grid grid-cols-3 gap-1">
                            {EFFECTS.map((e) => {
                                const on = e.id === design.effect;
                                return (
                                    <button
                                        key={e.id}
                                        title={e.hint}
                                        className="h-8 rounded-lg text-[12px] transition-colors duration-150"
                                        style={{ background: on ? withAlpha(scheme.fg, 0.14) : withAlpha(scheme.fg, 0.04), color: on ? t.text : t.sub, fontWeight: on ? 500 : 400 }}
                                        onClick={() => set({ effect: e.id as EffectId })}
                                    >
                                        {e.id === 'raw' && drawn ? 'As it is' : e.name}
                                    </button>
                                );
                            })}
                        </div>
                        <EffectControls design={design} set={set} scheme={scheme} />
                        <Label scheme={scheme}>Tone</Label>
                        <Field label="Brightness" scheme={scheme}>
                            <Slider scheme={scheme} value={design.brightness} min={-0.5} max={0.5} step={0.02} onChange={(brightness) => set({ brightness })} format={(v) => v.toFixed(2)} />
                        </Field>
                        <Field label="Contrast" scheme={scheme}>
                            <Slider scheme={scheme} value={design.contrast} min={0.3} max={3} step={0.05} onChange={(contrast) => set({ contrast })} format={(v) => v.toFixed(2)} />
                        </Field>
                        <Field label="Gamma" scheme={scheme}>
                            <Slider scheme={scheme} value={design.gamma} min={0.3} max={3} step={0.05} onChange={(gamma) => set({ gamma })} format={(v) => v.toFixed(2)} />
                        </Field>
                        <Field label="Saturation" scheme={scheme}>
                            <Slider scheme={scheme} value={design.saturation} min={0} max={2} step={0.05} onChange={(saturation) => set({ saturation })} format={(v) => v.toFixed(2)} />
                        </Field>
                        <Field label="Invert" scheme={scheme}>
                            <Switch scheme={scheme} on={design.invert} onChange={(invert) => set({ invert })} />
                        </Field>
                        <div className="h-4" />
                        <Label scheme={scheme}>Color</Label>
                        <div className="mb-2">
                            <Segmented
                                scheme={scheme}
                                value={design.color}
                                options={[{ value: 'scheme', label: 'Scheme' }, { value: 'source', label: 'Picture' }, { value: 'mono', label: 'One' }, { value: 'duotone', label: 'Two' }]}
                                onChange={(color) => set({ color })}
                            />
                        </div>
                        {design.color === 'mono' ? <Swatches value={design.mono} scheme={scheme} onPick={(mono) => set({ mono })} /> : null}
                        {design.color === 'duotone' ? (
                            <>
                                <div className="mb-1 text-[11.5px]" style={{ color: t.faint }}>Shadows</div>
                                <Swatches value={design.low} scheme={scheme} onPick={(low) => set({ low })} />
                                <div className="mb-1 text-[11.5px]" style={{ color: t.faint }}>Highlights</div>
                                <Swatches value={design.high} scheme={scheme} onPick={(high) => set({ high })} />
                            </>
                        ) : null}
                        <div className="h-4" />
                        <Label scheme={scheme}>Placement</Label>
                        <Field label="Strength" scheme={scheme}>
                            <Slider scheme={scheme} value={design.opacity} min={0.05} max={1} step={0.05} onChange={(opacity) => set({ opacity })} format={(v) => `${Math.round(v * 100)}%`} />
                        </Field>
                        <Field label="Fit" scheme={scheme}>
                            <Segmented scheme={scheme} value={design.fit} options={[{ value: 'cover', label: 'Fill' }, { value: 'contain', label: 'Fit' }]} onChange={(fit) => set({ fit })} />
                        </Field>
                        <div className="mb-1 mt-1 text-[12.5px]" style={{ color: withAlpha(scheme.fg, 0.75) }}>
                            Fade, so the text stays clear
                        </div>
                        <div className="mb-2 grid grid-cols-4 gap-1">
                            {(['none', 'radial', 'corner', 'left', 'right', 'top', 'bottom'] as Fade[]).map((fade) => (
                                <button
                                    key={fade}
                                    className="h-7 rounded-md text-[11.5px] capitalize"
                                    style={{ background: design.fade === fade ? withAlpha(scheme.fg, 0.14) : withAlpha(scheme.fg, 0.04), color: design.fade === fade ? t.text : t.sub }}
                                    onClick={() => set({ fade })}
                                >
                                    {fade === 'none' ? 'None' : fade}
                                </button>
                            ))}
                        </div>
                        {media?.kind === 'video' && !drawn ? (
                            <Field label="Speed" scheme={scheme}>
                                <Slider scheme={scheme} value={design.speed} min={0.25} max={3} step={0.25} onChange={(speed) => set({ speed })} format={(v) => `${v}×`} />
                            </Field>
                        ) : null}
                        <Field label="Show text over it" scheme={scheme}>
                            <Switch scheme={scheme} on={showText} onChange={setShowText} />
                        </Field>
                    </div>
                    <div className="flex shrink-0 gap-2 p-3" style={{ borderTop: `1px solid ${t.border}` }}>
                        <button
                            className="h-9 flex-1 rounded-lg text-[13px]"
                            style={{ background: withAlpha(scheme.fg, 0.06), color: t.sub }}
                            onClick={() => {
                                setDesign({ ...DEFAULT_DESIGN });
                                setScene({ ...DEFAULT_SCENE });
                            }}
                        >
                            Reset
                        </button>
                        <button
                            className="h-9 flex-[2] rounded-lg text-[13px] font-medium"
                            style={{ background: ready ? css(scheme.fg) : withAlpha(scheme.fg, 0.1), color: ready ? css(scheme.bg) : t.faint }}
                            onClick={apply}
                        >
                            Use as background
                        </button>
                    </div>
                </div>
            </div>
        </div>
    );
}

/** A drawn background's own settings: its colors, its speed and how much of it there is. */
function SceneControls(props: { scene: SceneParams; set: (patch: Partial<SceneParams>) => void; scheme: Scheme }) {
    const { scene, set, scheme } = props;
    const t = tokens(scheme);
    return (
        <>
            <div className="mb-1 text-[11.5px]" style={{ color: t.faint }}>Color</div>
            <Swatches value={scene.color} scheme={scheme} onPick={(color) => set({ color })} />
            <div className="mb-1 text-[11.5px]" style={{ color: t.faint }}>Second color</div>
            <Swatches value={scene.color2} scheme={scheme} onPick={(color2) => set({ color2 })} />
            <Field label="Speed" scheme={scheme}>
                <Slider scheme={scheme} value={scene.speed} min={0.1} max={3} step={0.1} onChange={(v) => set({ speed: Math.round(v * 10) / 10 })} format={(v) => `${v.toFixed(1)}×`} />
            </Field>
            <Field label="Amount" scheme={scheme}>
                <Slider scheme={scheme} value={scene.density} min={0.3} max={2.5} step={0.1} onChange={(v) => set({ density: Math.round(v * 10) / 10 })} format={(v) => `${v.toFixed(1)}×`} />
            </Field>
        </>
    );
}

/** What the chosen effect has of its own to tune. */
function EffectControls(props: { design: Design; set: (patch: Partial<Design>) => void; scheme: Scheme }) {
    const { design: d, set, scheme } = props;
    const e = d.effect;
    const hasCell = e !== 'raw';
    const hasThreshold = e === 'dither' || e === 'braille' || e === 'edges';
    const hasGap = e === 'pixel' || e === 'dots' || e === 'halftone' || e === 'lines' || e === 'dither';
    const hasRound = e === 'dither' || e === 'dots' || e === 'halftone';
    if (!hasCell && !hasThreshold) {
        return (
            <>
                <Field label="Detail" scheme={scheme}>
                    <Slider scheme={scheme} value={d.cell} min={3} max={30} step={1} onChange={(cell) => set({ cell })} format={(v) => `${v}`} />
                </Field>
                <div className="h-4" />
            </>
        );
    }
    return (
        <>
            <Label scheme={scheme}>{EFFECTS.find((x) => x.id === e)?.name}</Label>
            <Field label="Cell size" scheme={scheme}>
                <Slider scheme={scheme} value={d.cell} min={3} max={32} step={1} onChange={(cell) => set({ cell })} format={(v) => `${v} px`} />
            </Field>
            {e === 'ascii' ? (
                <>
                    <div className="mb-2 mt-1">
                        <Segmented
                            scheme={scheme}
                            value={d.charset}
                            options={[
                                { value: 'classic', label: 'Classic' }, { value: 'dense', label: 'Dense' }, { value: 'blocks', label: 'Blocks' },
                                { value: 'binary', label: '01' }, { value: 'kana', label: 'Kana' }, { value: 'custom', label: 'Own' },
                            ]}
                            onChange={(charset) => set({ charset: charset as Charset })}
                        />
                    </div>
                    {d.charset === 'custom' ? (
                        <input
                            value={d.customChars}
                            placeholder="Dark to light, like  .oO@"
                            className="mb-2 h-8 w-full rounded-lg px-3 text-[12.5px] outline-none"
                            style={{ background: withAlpha(scheme.fg, 0.07), color: css(scheme.fg), fontFamily: 'Cascadia Mono, Consolas, monospace' }}
                            onChange={(ev: any) => set({ customChars: typeof ev === 'string' ? ev : ev.target.value })}
                        />
                    ) : null}
                </>
            ) : null}
            {hasThreshold ? (
                <Field label="Threshold" scheme={scheme}>
                    <Slider scheme={scheme} value={d.threshold} min={0.05} max={0.95} step={0.05} onChange={(threshold) => set({ threshold })} format={(v) => v.toFixed(2)} />
                </Field>
            ) : null}
            {e === 'dither' ? (
                <Field label="Diffused" scheme={scheme}>
                    <Switch scheme={scheme} on={d.diffuse} onChange={(diffuse) => set({ diffuse })} />
                </Field>
            ) : null}
            {hasGap ? (
                <Field label="Gap" scheme={scheme}>
                    <Slider scheme={scheme} value={d.gap} min={0} max={0.8} step={0.05} onChange={(gap) => set({ gap })} format={(v) => `${Math.round(v * 100)}%`} />
                </Field>
            ) : null}
            {hasRound ? (
                <Field label="Round" scheme={scheme}>
                    <Switch scheme={scheme} on={d.round} onChange={(round) => set({ round })} />
                </Field>
            ) : null}
            {e === 'posterize' ? (
                <Field label="Tones" scheme={scheme}>
                    <Slider scheme={scheme} value={d.levels} min={2} max={8} step={1} onChange={(levels) => set({ levels })} format={(v) => `${v}`} />
                </Field>
            ) : null}
            <div className="h-4" />
        </>
    );
}

function Label(props: { scheme: Scheme; children: any }) {
    return (
        <div className="mb-2 text-[12px] font-medium" style={{ color: withAlpha(props.scheme.fg, 0.42) }}>
            {props.children}
        </div>
    );
}

function Swatches(props: { value: number; scheme: Scheme; onPick: (color: number) => void }) {
    return (
        <div className="mb-2 flex flex-wrap gap-1.5">
            {SWATCHES.map((c) => (
                <button
                    key={c}
                    aria-label={css(c)}
                    className="size-6 rounded-full"
                    style={{ background: css(c), boxShadow: props.value === c ? `0 0 0 2px ${css(props.scheme.bg)}, 0 0 0 3.5px ${css(props.scheme.fg)}` : `inset 0 0 0 1px ${withAlpha(0xffffff, 0.1)}` }}
                    onClick={() => props.onPick(c)}
                />
            ))}
        </div>
    );
}

/** The design drawn live on a canvas the size of the preview, moving for a video or a drawn background. */
function Preview(props: { make: () => FrameSource | null; sourceKey: string; design: Design; scheme: Scheme }) {
    const box = useRef<HTMLDivElement>(null);
    const canvas = useRef<HTMLCanvasElement>(null);
    const latest = useRef(props);
    latest.current = props;
    const [size, setSize] = useState({ w: 800, h: 500 });
    useEffect(() => {
        const element = box.current;
        if (!element) return;
        const measure = () => {
            const r = element.getBoundingClientRect();
            setSize({ w: Math.max(1, Math.round(r.width)), h: Math.max(1, Math.round(r.height)) });
        };
        measure();
        const observer = new ResizeObserver(measure);
        observer.observe(element);
        return () => observer.disconnect();
    }, []);
    useEffect(() => {
        const c = canvas.current;
        if (!c) return;
        c.width = size.w;
        c.height = size.h;
        const ctx = c.getContext('2d')!;
        const source = props.make();
        if (!source) return;
        const start = Date.now();
        let timer: ReturnType<typeof setTimeout> | null = null;
        const draw = () => {
            const { design, scheme } = latest.current;
            const t = (Date.now() - start) / 1000;
            ctx.setTransform(1, 0, 0, 1, 0, 0);
            ctx.clearRect(0, 0, size.w, size.h);
            drawDesign(ctx, source, design, size.w, size.h, { fg: scheme.fg, accent: scheme.ansi[4] }, t);
            const fps = source.fps || (design.effect === 'matrix' ? 12 : 0);
            if (fps) timer = setTimeout(draw, 1000 / Math.min(30, fps));
        };
        draw();
        return () => {
            if (timer !== null) clearTimeout(timer);
        };
    }, [size.w, size.h, props.sourceKey, JSON.stringify(props.design)]);
    return (
        <div ref={box} className="absolute inset-0">
            <canvas ref={canvas} style={{ width: '100%', height: '100%', display: 'block' }} />
        </div>
    );
}
