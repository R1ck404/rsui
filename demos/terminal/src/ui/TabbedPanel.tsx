// The active tab and its panel share one outline and one background. The frame shows through around them.

import { cloneElement, useLayoutEffect, useRef, useState } from 'react';
import type { ReactElement } from 'react';
import { Path, Svg } from 'rsui';
import { Icon } from './Icon';
import type { IconName } from './Icon';
import { IconButton } from './controls';
import { useReorder } from './reorder';
import type { ReorderOutside, TabDragProps } from './reorder';
import { css, withAlpha, tokens } from '../themes';
import type { Scheme } from '../themes';

export interface PanelTab {
    key: string | number;
    title: string;
    /** A page's icon; a shell has none. */
    icon?: IconName;
    active: boolean;
    /** Something running in it, or that failed, shown as a small dot. Nothing for a shell that's waiting. */
    status?: 'busy' | 'failed' | 'attention' | null;
    onPick: () => void;
    onClose?: () => void;
}

/** The width of the active tab and which corners it meets. */
export interface PanelPlace {
    squareLeft: boolean;
    squareRight: boolean;
    tabWidth: number;
}

/** The height of the row of tabs. */
export const TAB_ROW = 38;
const TAB_TOP_INSET = 6;
export const TAB_WIDTH = 200;
const TAB_MIN = 120;
const SCROLL_BUTTON = 24;
const SCROLL_CONTROLS = SCROLL_BUTTON * 2;

/** Traces the upper edge of the panel through the active tab, including its flared corners. */
function panelOutline(w: number, h: number, radius: number, start: number, end: number) {
    const y = TAB_ROW;
    const r = Math.min(radius, w / 2, (h - y) / 2);
    const left = Math.min(r, start);
    const right = Math.min(r, w - end);
    const tab = end > start;
    let top = `M0 ${y + (tab ? left : r)} Q0 ${y} ${tab ? left : r} ${y}`;
    if (tab) {
        const t = Math.min(12, Math.max(4, radius), (end - start) / 2);
        const l = Math.min(t, start - left);
        const k = Math.min(t, w - end - right);
        top = (start === 0 ? `M0 ${t}` : `${top} H${start - l} Q${start} ${y} ${start} ${y - l}`)
            + ` V${t} Q${start} 0 ${start + t} 0 H${end - t} Q${end} 0 ${end} ${t}`
            + (end === w ? '' : ` V${y - k} Q${end} ${y} ${end + k} ${y} H${w - right} Q${w} ${y} ${w} ${y + right}`);
    } else {
        top += ` H${w - r} Q${w} ${y} ${w} ${y + r}`;
    }
    return `${top} V${h - r} Q${w} ${h} ${w - r} ${h} H${r} Q0 ${h} 0 ${h - r} Z`;
}

export function TabbedPanel(props: {
    scheme: Scheme;
    tabs: PanelTab[];
    onReorder?: (from: number, to: number) => void;
    tabDock?: ReorderOutside;
    /** The panel's color, which the open tab is too. */
    panelColor: string;
    radius: number;
    /** The shared background, clipped to the joined tab and panel. */
    panelBackground?: (outline: string) => any;
    /** What goes before the tabs, `leadWidth` wide, as the workspace's button. */
    lead?: any;
    leadWidth?: number | ((panelWidth: number) => number);
    /** What goes after the tabs, as a + button. */
    after?: any;
    /** What goes at the right end of the row, as the window's buttons. */
    right?: any;
    /** The panel itself, with its joined corners supplied here. */
    children: ReactElement<{ joined?: PanelPlace }>;
    /** Room around it: left, right and bottom. */
    inset?: { left: number | ((panelWidth: number) => number); right: number; bottom: number };
}) {
    const { scheme } = props;
    const [dragging, setDragging] = useState(false);
    const reorder = useReorder(props.onReorder ? props.tabs.map((tab) => tab.key) : [], 'X',
        (from, to) => props.onReorder?.(from, to), setDragging, props.tabDock);
    const canReorder = !!props.onReorder && props.tabs.length > 1;
    const inset = props.inset ?? { left: 6, right: 6, bottom: 6 };
    const box = useRef<HTMLDivElement>(null);
    const after = useRef<HTMLDivElement>(null);
    const toolbar = useRef<HTMLDivElement>(null);
    const [size, setSize] = useState({ w: 1000, h: 700, after: 32, toolbar: 240 });
    const [scroll, setScroll] = useState(0);
    useLayoutEffect(() => {
        const element = box.current;
        const actions = toolbar.current;
        const newTab = after.current;
        if (!element || !actions || !newTab) return;
        const measure = () => {
            const r = element.getBoundingClientRect();
            const w = Math.floor(r.width);
            const h = Math.round(r.height);
            const after = Math.ceil(newTab.getBoundingClientRect().width);
            const toolbar = Math.ceil(actions.getBoundingClientRect().width);
            setSize((was) => w === was.w && h === was.h && after === was.after && toolbar === was.toolbar ? was : { w, h, after, toolbar });
        };
        measure();
        const observer = new ResizeObserver(measure);
        observer.observe(element);
        observer.observe(actions);
        observer.observe(newTab);
        return () => observer.disconnect();
    }, []);

    const insetLeft = typeof inset.left === 'function' ? inset.left(size.w) : inset.left;
    const W = Math.max(1, size.w - insetLeft - inset.right);
    const H = Math.max(TAB_ROW, size.h - TAB_TOP_INSET - inset.bottom);
    const available = Math.max(0, W - size.toolbar - size.after);
    const n = props.tabs.length;
    const leadLimit = Math.max(0, available - (n ? TAB_MIN : 0));
    const leadWidth = typeof props.leadWidth === 'function' ? props.leadWidth(size.w) : props.leadWidth ?? 0;
    const preferredLead = Math.min(props.lead ? leadWidth : 0, leadLimit);
    const overflow = n > 0 && n * TAB_MIN > available - preferredLead;
    const scrollControls = overflow ? SCROLL_CONTROLS : 0;
    const lead = Math.min(preferredLead, Math.max(0, leadLimit - scrollControls));
    const tabSpace = Math.max(0, available - lead - scrollControls);
    const tabW = overflow ? TAB_MIN : Math.min(TAB_WIDTH, Math.floor(tabSpace / Math.max(1, n)));
    const viewportWidth = Math.min(tabSpace, n * tabW);
    const maxScroll = Math.max(0, n * tabW - viewportWidth);
    const activeIndex = props.tabs.findIndex((tab) => tab.active);
    const activeKey = props.tabs[activeIndex]?.key;
    // The tabs and the shared panel outline use the same horizontal offset.
    const offset = Math.min(scroll, maxScroll);
    const revealTab = (index: number, current: number) => {
        if (index < 0 || maxScroll === 0) return 0;
        const left = index * tabW;
        const right = left + tabW;
        return Math.min(maxScroll, left, Math.max(current, right - viewportWidth));
    };
    useLayoutEffect(() => {
        setScroll((current) => revealTab(activeIndex, current));
    }, [activeKey, activeIndex, maxScroll, tabW, viewportWidth]);
    const moveTabs = (distance: number) => setScroll((current) => Math.max(0, Math.min(maxScroll, current + distance)));

    const tabLeft = activeIndex * tabW - offset;
    const visibleLeft = Math.max(0, tabLeft);
    const visibleRight = Math.min(viewportWidth, tabLeft + tabW);
    const joined = activeIndex >= 0 && !dragging && visibleRight > visibleLeft;
    const start = joined ? lead + visibleLeft : W;
    const end = joined ? lead + visibleRight : W;
    const outline = panelOutline(W, H, props.radius, start, end);
    const panel: PanelPlace = { squareLeft: start === 0, squareRight: start < end && end === W, tabWidth: tabW };

    return (
        <div ref={box} className="relative size-full">
            <div className="absolute" style={{ left: insetLeft, right: inset.right, top: TAB_TOP_INSET, bottom: inset.bottom }}>
                {activeIndex >= 0 ? (
                    <div className="pointer-events-none absolute inset-0">
                        <Svg viewBox={`0 0 ${W} ${H}`} width={W} height={H} fill={props.panelColor} stroke="none">
                            <Path d={outline} />
                        </Svg>
                        {props.panelBackground?.(outline)}
                    </div>
                ) : null}
                <div className="absolute inset-x-0 bottom-0" style={{ top: TAB_ROW }}>
                    {cloneElement(props.children, { joined: panel })}
                </div>
            </div>
            <div className="absolute inset-x-0 top-0 [app-region:drag]" style={{ height: TAB_TOP_INSET + TAB_ROW }}>
                <div className="absolute bottom-0 flex items-center" style={{ left: insetLeft, height: TAB_ROW }}>
                    {lead > 0 ? (
                        <div className="flex h-full min-w-0 shrink-0 items-center pl-1" style={{ width: lead }}>
                            {props.lead}
                        </div>
                    ) : null}
                    <div className={`h-full shrink-0 ${overflow ? 'overflow-hidden' : ''}`} style={{ width: viewportWidth }}
                        onWheel={overflow ? (event) => {
                            const delta = event.deltaX || event.deltaY;
                            if (event.ctrlKey || !delta) return;
                            event.preventDefault();
                            moveTabs(delta * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? viewportWidth : 1));
                        } : undefined}>
                        <div role="tablist" aria-label="Open tabs" className="relative flex h-full" style={{ width: n * tabW, left: -offset }}>
                            {props.tabs.map((tab, i) => (
                                <div key={tab.key} className="h-full shrink-0" style={{ width: tabW }}
                                    onFocus={overflow ? () => setScroll((current) => revealTab(i, current)) : undefined}>
                                    <TabLabel tab={tab} scheme={scheme} drag={canReorder ? reorder.tab(i, tab.onPick, () => tab.onClose?.()) : undefined}
                                        activeFill={dragging ? props.panelColor : undefined} />
                                </div>
                            ))}
                        </div>
                    </div>
                    {overflow ? (
                        <div className="flex shrink-0">
                            <IconButton icon="back" label="Scroll tabs left" scheme={scheme} size={SCROLL_BUTTON} keepFocus noDrag
                                disabled={offset <= 0} onClick={() => moveTabs(-tabW)} />
                            <IconButton icon="forward" label="Scroll tabs right" scheme={scheme} size={SCROLL_BUTTON} keepFocus noDrag
                                disabled={offset >= maxScroll} onClick={() => moveTabs(tabW)} />
                        </div>
                    ) : null}
                    <div ref={after} className="shrink-0 pl-1">{props.after}</div>
                </div>
                <div ref={toolbar} className="absolute right-0 top-0 flex h-full items-center">{props.right}</div>
            </div>
        </div>
    );
}

function TabLabel(props: { tab: PanelTab; scheme: Scheme; drag?: TabDragProps; activeFill?: string }) {
    const { tab, scheme } = props;
    const t = tokens(scheme);
    const [hover, setHover] = useState(false);
    const dot = tab.status === 'attention' ? scheme.ansi[5] : tab.status === 'busy' ? scheme.ansi[3] : tab.status === 'failed' ? scheme.ansi[1] : null;
    return (
        <div
            className={`group/tab relative flex h-full items-center gap-2 pl-4 pr-2 [app-region:no-drag] ${props.drag ? 'reorder-item reorder-tab touch-none' : 'cursor-default'}`}
            style={{ color: tab.active ? t.text : hover ? withAlpha(scheme.fg, 0.75) : t.faint }}
            onPointerEnter={() => setHover(true)}
            onPointerLeave={() => setHover(false)}
            onPointerDown={(e: any) => {
                if (e.button === 1) tab.onClose?.();
                else tab.onPick();
            }}
            {...props.drag}
        >
            {/* A hovered tab that isn't open gets a soft fill inside the row, which never touches the panel. */}
            {!tab.active && hover ? <div className="absolute inset-x-1 inset-y-[5px] rounded-lg" style={{ background: t.hover }} /> : null}
            {tab.active && props.activeFill ? <div className="absolute inset-0 rounded-t-xl" style={{ background: props.activeFill }} /> : null}
            {dot !== null ? <div className="relative size-[6px] shrink-0 rounded-full" style={{ background: css(dot) }} /> : null}
            {tab.icon ? (
                <span className="relative flex shrink-0" style={{ color: tab.active ? t.sub : 'inherit' }}>
                    <Icon name={tab.icon} size={13} />
                </span>
            ) : null}
            <span className="relative min-w-0 flex-1 truncate text-[13px]" style={{ fontWeight: tab.active ? 500 : 400 }}>
                {tab.title}
            </span>
            <button type="button" role="tab" aria-selected={tab.active} aria-label={tab.title}
                className="absolute inset-0 rounded-lg bg-transparent focus-visible:outline-2 focus-visible:-outline-offset-2"
                onClick={(event) => { if (event.detail === 0) tab.onPick(); }} />
            {tab.onClose ? (
                <button
                    type="button"
                    aria-label={`Close ${tab.title}`}
                    className={`relative flex size-5 shrink-0 items-center justify-center rounded-md transition-colors duration-150 hover:bg-white/10 focus-visible:outline-2 ${tab.active || hover ? '' : 'opacity-0 group-focus-within/tab:opacity-100'}`}
                    style={{ color: t.sub }}
                    onPointerDown={(e: any) => e.stopPropagation()}
                    onClick={tab.onClose}
                >
                    <Icon name="close" size={11} strokeWidth={1.7} />
                </button>
            ) : null}
        </div>
    );
}
