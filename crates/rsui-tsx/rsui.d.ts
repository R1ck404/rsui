// Types for `import ... from 'rsui'` and for JSX, for editors and for `tsc`. Running an app never reads this: the engine
// removes types as it parses them.

declare module 'rsui' {
    export type Child = Element | string | number | boolean | null | undefined | Child[];
    /** What JSX gives. Its parts are the runtime's business. */
    export interface Element {
        readonly type: unknown;
        readonly key: unknown;
    }
    export type Component<P = {}> = (props: P) => Child;
    /**
     * What `onClick` is given. Where the pointer was is from the window's top left, in its pixels, and `offsetX` and
     * `offsetY` are from the top left of what was clicked. With the keyboard, or an agent that clicks something by what
     * it's called, it's the middle of it, and `detail` is 0. `stopPropagation()` keeps it from `document` and `window`,
     * which hear clicks too if `rsui/web` is loaded. A click on rsui's own elements stops where it lands either way.
     */
    export interface ClickEvent {
        readonly type: 'click';
        readonly defaultPrevented: boolean;
        preventDefault(): void;
        isDefaultPrevented(): boolean;
        stopPropagation(): void;
        isPropagationStopped(): boolean;
        persist(): void;
        /** 0 for the main button. */
        readonly button: number;
        /** Which buttons are down: 1 the main one, 2 the other, 4 the middle. */
        readonly buttons: number;
        /** How many clicks in a row this is: 2 for the second of a double click, and 0 from the keyboard. */
        readonly detail: number;
        readonly clientX: number;
        readonly clientY: number;
        readonly pageX: number;
        readonly pageY: number;
        readonly screenX: number;
        readonly screenY: number;
        readonly offsetX: number;
        readonly offsetY: number;
        readonly ctrlKey: boolean;
        readonly shiftKey: boolean;
        readonly altKey: boolean;
        /** The Windows key. */
        readonly metaKey: boolean;
        getModifierState(key: 'Control' | 'Shift' | 'Alt' | 'Meta' | string): boolean;
        /** On rsui's own elements, sends the click on to the nearest `onClick` around this one, which it doesn't go to otherwise. */
        passOn(): void;
        readonly target: { focus(): void; blur(): void };
        readonly currentTarget: { focus(): void; blur(): void };
        readonly nativeEvent: unknown;
    }

    export const Fragment: Component<{ children?: Child }>;

    export function useState<T>(initial: T | (() => T)): [T, (next: T | ((old: T) => T)) => void];
    export function useReducer<S, A>(reducer: (state: S, action: A) => S, initial: S): [S, (action: A) => void];
    export function useReducer<S, A, I>(
        reducer: (state: S, action: A) => S,
        initial: I,
        init: (initial: I) => S,
    ): [S, (action: A) => void];
    export function useRef<T>(initial: T): { current: T };
    /** For something a `ref` will hold, as `useRef<HTMLInputElement>(null)`. */
    export function useRef<T>(initial: T | null): { current: T | null };
    export function useMemo<T>(compute: () => T, deps?: readonly unknown[]): T;
    export function useCallback<T extends Function>(callback: T, deps?: readonly unknown[]): T;
    /** Runs after the tree is up to date, and whenever `deps` change. What it returns runs before the next time. */
    export function useEffect(run: () => void | (() => void), deps?: readonly unknown[]): void;
    export const useLayoutEffect: typeof useEffect;
    /**
     * Which of the system's layers a window is in. `normal` is with other windows. `top` is above other programs' windows, as
     * `alwaysOnTop` is. `bottom` is beneath every other window, staying there when it's clicked, which doesn't take the keyboard
     * unless what's clicked takes keys; Show Desktop (Win+D) hides it with the rest. `desktop` is on the desktop itself, behind
     * its icons and over the wallpaper, where Show Desktop leaves it; the icons are over it, so the pointer and keys don't reach
     * it, which suits a wallpaper or a clock.
     */
    export type WindowLayer = 'normal' | 'top' | 'bottom' | 'desktop';
    /**
     * A strip along an edge of a screen that a window keeps for itself, as the taskbar does: other windows maximize beside it.
     * `size` is how thick it is in logical pixels, and `monitor` which screen, from 0 for the main one, which it is otherwise.
     */
    export interface AppBarOptions {
        edge: 'top' | 'bottom' | 'left' | 'right';
        size: number;
        monitor?: number;
    }
    /** What can be done to a window that's open. */
    export interface WindowControls {
        readonly id: number;
        /** No title bar or edges of its own. `dragsWindow` on what's in it says where it moves from. */
        setFrameless(on?: boolean): void;
        /** Stays above other programs' windows. The same as `setLayer('top')`, and taking it away only moves a window that's on top. */
        setAlwaysOnTop(on?: boolean): void;
        /** Into one of the system's layers. See `WindowLayer`. */
        setLayer(layer?: WindowLayer): void;
        /**
         * Only what's drawn in it shows, at the opacity it's drawn with, with what's behind it everywhere else: leave the root's
         * `background` out, or make it `transparent`. Where nothing's drawn, and nothing takes a click, the pointer goes through to
         * what's behind.
         */
        setTransparent(on?: boolean): void;
        /** Keeps a strip along an edge of a screen for itself and moves there, or gives it back with `null`. */
        setAppBar(bar: AppBarOptions | null): void;
        /** Shows on every virtual desktop. It's then a tool window, which is left off the taskbar and Alt+Tab too. */
        setAllDesktops(on?: boolean): void;
        setFullscreen(on?: boolean): void;
        minimize(): void;
        maximize(): void;
        /** Back from being minimized or maximized. */
        restore(): void;
        /** Off the screen and the taskbar, while it carries on running, as for something that lives in the tray. */
        hide(): void;
        show(): void;
        /** To the front. The system only allows this if this program is what's being used, and otherwise it flashes in the taskbar. */
        focus(): void;
        /** To the middle of the screen it's on. */
        center(): void;
        moveTo(x: number, y: number): void;
        /** To this much room inside it. */
        resize(width: number, height: number): void;
        /** The text in its title bar and on its taskbar button. The main window can also start with `export const title`. */
        setTitle(title: string): void;
        /** Puts a menu bar on it, or takes it off with `null`. Only the main window can have one yet. */
        setMenu(menu: import('rsui/desktop').MenuItem[] | null): void;
        /** Closes it. For the main window, that ends the program, whatever its `onClose` would have said. */
        close(): void;
        /** From 0, nothing, to 1, all of it. */
        setOpacity(value: number): void;
        /** Whether its edges can be dragged and it can be maximized. */
        setResizable(on?: boolean): void;
        /** How small and how big it can be made inside, in logical pixels, each if that's limited. */
        setSizeLimits(limits: { minWidth?: number; minHeight?: number; maxWidth?: number; maxHeight?: number }): void;
        /** Left off the taskbar and Alt+Tab. */
        setSkipTaskbar(on?: boolean): void;
        /** An `.ico` file or a picture, or one's bytes. */
        setIcon(icon: string | Uint8Array): void;
        /** The pointer goes straight through it, to what's behind. */
        setClickThrough(on?: boolean): void;
        /** Left out of screenshots and recordings. */
        setContentProtection(on?: boolean): void;
        /** Shown without taking the keyboard. */
        showWithoutActivation(): void;
        /** Starts resizing from an edge, as if it were dragged, for a window with no frame, while a button's held. */
        startResize(edge?: 'left' | 'right' | 'top' | 'bottom' | 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right'): void;
        /** How it is now, in logical pixels. */
        getState(): WindowState;
        /** Runs with how it is each time that changes. What it gives back stops it. */
        onStateChange(run: (state: WindowState) => void): () => void;
        /** What's in it, as a PNG file's bytes. */
        capture(): Promise<Uint8Array>;
        /**
         * Up to seven buttons under its picture in the taskbar, which shows when the pointer's over its button, in place of those
         * that were there. `[]` takes them away. `thumb ID` clicks one in steps.
         */
        setThumbnailButtons(buttons: { id?: string | number; icon?: string | Uint8Array; tooltip?: string; disabled?: boolean; onClick?: () => void }[]): void;
        /**
         * What the system draws behind it, as Windows 11's own programs have, with the title bar dark or light as the system's
         * colors are. Inside, it shows wherever nothing's drawn: leave the root's `background` out, or make it `transparent`.
         * It can be set, changed or taken away (`'none'`) at any time. Windows 10 doesn't have backdrops, and leaves the window as
         * it was.
         */
        setBackdrop(kind?: 'mica' | 'acrylic' | 'tabbed' | 'none'): void;
    }
    export interface WindowState {
        title: string;
        x: number;
        y: number;
        width: number;
        height: number;
        visible: boolean;
        minimized: boolean;
        maximized: boolean;
        fullscreen: boolean;
        focused: boolean;
    }
    /** How the window this component's in is, which it renders again with as that changes. */
    export function useWindowState(): WindowState;
    /**
     * The first window, which the program ends with. It starts with no frame, on top or fullscreen if the main file says so, with
     * `export const frameless = true`, `export const alwaysOnTop = true` or `export const fullscreen = true`, and the same for
     * `export const layer = 'desktop'`, `transparent`, `skipTaskbar`, `allDesktops` and `appBar = { edge: 'top', size: 32 }`.
     */
    export const mainWindow: WindowControls;
    /** The window that this component is in. */
    export function useCurrentWindow(): WindowControls;
    /** A number that moves smoothly to `target`, over `duration` milliseconds. */
    export function useAnimated(target: number, duration?: number): number;

    export interface Context<T> {
        Provider: Component<{ value: T; children?: Child }>;
    }
    export function createContext<T>(fallback: T): Context<T>;
    export function useContext<T>(context: Context<T>): T;

    /** Takes over the window while `open`, with the rest dimmed behind it. */
    export const Dialog: Component<{
        id?: string;
        name?: string;
        open: boolean;
        onClose?: () => void;
        children?: Child;
    }>;
    /** A box that's ticked or not. `label` shows beside it unless `showLabel` is false, and it's what it's called either way. */
    export const Checkbox: Component<Omit<ElementProps, 'onChange'> & {
        id?: string;
        checked: boolean;
        label: string;
        showLabel?: boolean;
        onChange?: (checked: boolean) => void;
    }>;
    /** A value between `min` and `max`, set by dragging, by clicking along it, or with the arrow keys. */
    export const Slider: Component<{
        id: string;
        value: number;
        min?: number;
        max?: number;
        step?: number;
        width?: number;
        name?: string;
        onChange?: (value: number) => void;
    }>;
    /** A button that opens a menu. */
    export const MenuButton: Component<{
        id?: string;
        label: string;
        items: { label: string; onSelect?: () => void }[];
    }>;
    /** A button showing which of `options` is chosen, which opens a menu of them. */
    export const Select: Component<{
        id?: string;
        options: string[];
        value: number;
        onChange: (index: number, option: string) => void;
    }>;
    /** An input with a border. Give it `rows` for several lines. With `autoFocus` and a `key`, it takes the keyboard when it first shows. */
    export const TextInput: Component<ElementProps & {
        value?: string;
        placeholder?: string;
        /** The color of `placeholder`, which is a grey otherwise. */
        placeholderColor?: string;
        rows?: number;
        autoFocus?: boolean;
        /** Shows dots rather than what's typed, and doesn't let it be copied. */
        password?: boolean;
        /** Can be selected and copied, but not changed. */
        readOnly?: boolean;
        /** The most characters that can be put in it. */
        maxLength?: number;
    }>;
    /**
     * Another window, for as long as this is here: `{open && <Window title="Settings" onClose={() => setOpen(false)}>...</Window>}`.
     * What's inside it is part of this app, with the same state and context. Other props go on its `window`.
     */
    export const Window: Component<Omit<ElementProps, 'onClose'> & {
        title?: string;
        /** How much room there is inside it. */
        size?: [number, number];
        /** How small it can be made. */
        minSize?: [number, number];
        /** Whether the main window can't be used until this has gone. */
        modal?: boolean;
        /** Whether it stays above the main window, and goes down with it. It does unless this says otherwise. */
        owned?: boolean;
        /** No title bar or edges of its own, but still the system's shadow, and round corners on Windows 11. `dragsWindow` on what's in it says where it moves from. */
        frameless?: boolean;
        /** Stays above other programs' windows. The same as `layer="top"`, which `layer` says otherwise. */
        alwaysOnTop?: boolean;
        /** Which of the system's layers it's in. See `WindowLayer`. */
        layer?: WindowLayer;
        /** Only what's drawn in it shows, and the pointer goes through the rest. See `WindowControls.setTransparent`. */
        transparent?: boolean;
        /** Left off the taskbar and Alt+Tab. */
        skipTaskbar?: boolean;
        /** Shows on every virtual desktop, as a tool window, which is left off the taskbar too. */
        allDesktops?: boolean;
        /** Keeps a strip along an edge of a screen, as the taskbar does. `null` or nothing is none. */
        appBar?: AppBarOptions | null;
        /** Covers the whole screen. */
        fullscreen?: boolean;
        /** Its close button was clicked. With this, it stays open until it's taken away, and without it, it closes. */
        onClose?: () => void;
        /** Where it was the last time, which it opens at, and keeps from then on, under this name in the app's own store. */
        rememberPlacement?: string;
    }>;
    /** A switch that's on or off. `label` shows beside it, and it's what it's called either way. */
    export const Switch: Component<Omit<ElementProps, 'onChange'> & {
        id?: string;
        checked: boolean;
        label?: string;
        disabled?: boolean;
        onChange?: (checked: boolean) => void;
    }>;
    /** One of a few choices, of which `value` is the number of the one that's chosen. The arrow keys move between them. */
    export const RadioGroup: Component<{
        id?: string;
        options: string[];
        value: number;
        name?: string;
        disabled?: boolean;
        direction?: 'row' | 'column';
        gap?: number;
        onChange?: (index: number, option: string) => void;
    }>;
    /** How far something's got, from 0 to `max`, which is 1 unless it says otherwise. */
    export const ProgressBar: Component<{ id?: string; value: number; max?: number; width?: number | string; name?: string }>;
    /** Something's going on, for however long it takes. */
    export const Spinner: Component<{ size?: number; color?: string; name?: string }>;
    /** A row of tabs, of which `value` is the number of the one that's chosen. What goes with each is up to what's using this. */
    export const Tabs: Component<{
        id?: string;
        options: string[];
        value: number;
        gap?: number;
        onChange?: (index: number, option: string) => void;
    }>;
    /** How something inside an `Svg` is drawn. On the `Svg`, or a `G`, this is for all that's in it. */
    interface ShapeLook {
        /** `currentColor` is the `Svg`'s `color`, and `none` is nothing. It's `currentColor` if nothing says. */
        fill?: string | number;
        stroke?: string | number;
        strokeWidth?: number;
        strokeLinecap?: 'butt' | 'round' | 'square';
        /** How a line's corners are drawn. `miter` if nothing says, as SVG has it. */
        strokeLinejoin?: 'miter' | 'round' | 'bevel';
        /** How far a mitred corner may reach, in widths of the line, before it's bevelled instead: 4 if nothing says. */
        strokeMiterlimit?: number;
        /** Dashes and the gaps between them, as `'4 2'` or `[4, 2]`, starting `strokeDashoffset` along. */
        strokeDasharray?: string | number[];
        strokeDashoffset?: number;
        /** As SVG writes it: `'translate(4 2) rotate(45)'`, on this and what's in it. */
        transform?: string;
    }
    /** A shape: `<Svg viewBox="0 0 24 24"><Path d="M2 2H22V22Z" /></Svg>`, drawn once and kept as a picture. */
    export function Svg(props: ElementProps & ShapeLook & {
        viewBox?: string;
        /** Fills all the room it's been given, rather than keeping its shape, which is what a chart wants. */
        stretch?: boolean;
        children?: Child;
    }): Child;
    export function G(props: ShapeLook & { children?: Child }): Child;
    /** An SVG path: M, L, H, V, C, S, Q, T, A and Z, or those in lower case for going from the last point. */
    export function Path(props: ShapeLook & { d: string; key?: string | number }): Child;
    export function Line(props: ShapeLook & { x1: number; y1: number; x2: number; y2: number; key?: string | number }): Child;
    export function Polyline(props: ShapeLook & { points: string | [number, number][]; key?: string | number }): Child;
    export function Polygon(props: ShapeLook & { points: string | [number, number][]; key?: string | number }): Child;
    /** `rx` and `ry` round its corners, across and down: either is the other if only one's said. */
    export function Rect(props: ShapeLook & { x?: number; y?: number; width: number; height: number; rx?: number; ry?: number; key?: string | number }): Child;
    export function Circle(props: ShapeLook & { cx: number; cy: number; r: number; key?: string | number }): Child;
    export function Ellipse(props: ShapeLook & { cx: number; cy: number; rx: number; ry: number; key?: string | number }): Child;
    /** Whether the system's set to dark or light, which changes along with it, along with the title bar. */
    export function useColorScheme(): 'light' | 'dark';
    /** Gives the keyboard to the element with this key, once what's happening now has been applied. */
    export function focus(key: string | number): void;

    /**
     * Where children go across the direction they stack in. `baseline` lines up the first line of text in each, as what's on a line of
     * a page is, with anything that has no text lined up by its bottom edge. It's only for a row that wraps, and is `start` anywhere else.
     */
    export type Align = 'stretch' | 'start' | 'center' | 'end' | 'baseline';
    export type Justify = 'start' | 'center' | 'end' | 'space_between' | 'space_around' | 'space_evenly';
    export type Color = string | number;

    /** How an element is laid out and painted. These can be props of their own, or go in `style`. */
    export interface Style {
        direction?: 'row' | 'column';
        /** In pixels, or a string like `'50%'`. */
        width?: number | string;
        height?: number | string;
        minWidth?: number;
        maxWidth?: number;
        minHeight?: number;
        maxHeight?: number;
        /** Room outside this, in its parent, which can be less than nothing to overlap what's next to it. */
        margin?: number;
        marginX?: number;
        marginY?: number;
        marginTop?: number;
        marginRight?: number;
        marginBottom?: number;
        marginLeft?: number;
        /** With `absolute`, this takes up no room, and goes where `top`, `left`, `right` and `bottom` say, in its parent. */
        position?: 'relative' | 'absolute';
        /** In pixels, or a string like `'50%'`. With `left` and `right` both, and no `width`, that's how wide it is. */
        top?: number | string;
        left?: number | string;
        right?: number | string;
        bottom?: number | string;
        /** Moves this from where it would be, without moving what's around it. A string like `'-50%'` is of its own size. */
        /** Bigger or smaller than it was laid out, around its middle, with nothing else moving to make room. */
        scale?: number;
        scaleX?: number;
        scaleY?: number;
        /** Turned clockwise, in degrees. */
        rotate?: number;
        /** As CSS writes it: `'rotate(45deg) scale(1.2)'`, with `translate`, `scale`, `rotate`, `skew` and `matrix`. */
        transform?: string;
        /** What it turns around, as CSS writes it: `'top left'` or `'10px 50%'`. It's the middle otherwise. */
        transformOrigin?: string;
        translateX?: number | string;
        translateY?: number | string;
        /** With `grid`, children go in rows and columns, as `columns` and `rows` say. */
        display?: 'flex' | 'grid';
        /** As CSS writes them: `'1fr 200px auto'`, `'repeat(3, 1fr)'`, `'minmax(100px, 1fr) 50%'`. */
        columns?: string;
        rows?: string | number;
        /** Where this goes in a grid, as CSS writes it: `2`, `'span 2'`, `'1 / 3'`, `'1 / -1'`. */
        gridColumn?: number | string;
        gridRow?: number | string;
        /** Space between a grid's columns or rows, if it isn't `gap`. */
        columnGap?: number;
        rowGap?: number;
        /** Where children go across their cells in a grid. `align` is the other way. */
        justifyItems?: Align;
        /** `'column'` puts what doesn't say where it goes down each column, with a new column for each, rather than along each row. */
        gridFlow?: 'row' | 'column';
        /** Where a grid's rows go in room they don't fill, as CSS's `align-content` says. `justify` is the same for its columns. Either stops tracks sized by what's in them from stretching. */
        alignContent?: 'start' | 'center' | 'end' | 'space-between' | 'space-around' | 'space-evenly' | 'stretch';
        /** On an `<image>`: it sits on a line of text, as a picture on a page does. A group with `collapseMargins` that has only these in it leaves room below them for what goes under a line in its font. */
        onLine?: boolean;
        /** Takes a share of the parent's space. */
        flex?: number;
        grow?: number;
        shrink?: number;
        gap?: number;
        padding?: number;
        paddingX?: number;
        paddingY?: number;
        paddingTop?: number;
        paddingRight?: number;
        paddingBottom?: number;
        paddingLeft?: number;
        align?: Align;
        alignSelf?: Align;
        justify?: Justify;
        overlay?: 'below' | 'above' | 'right' | 'left' | 'center' | 'fill' | 'at';
        overlayX?: number;
        overlayY?: number;
        background?: Color;
        hover?: Color;
        pressed?: Color;
        /** The color of text. */
        color?: Color;
        borderColor?: Color;
        borderWidth?: number;
        radius?: number;
        opacity?: number;
        fontSize?: number;
        fontWeight?: string;
        /** A font by name, like `'Consolas'` or `'Segoe Fluent Icons'`, or a path to a font file. */
        fontFamily?: string;
        fontStyle?: 'normal' | 'italic';
        underline?: boolean;
        /** Where lines of text go. It's the side they start from otherwise. */
        textAlign?: 'left' | 'center' | 'right';
        /** Keeps text to one line, with `...` if it doesn't all fit. */
        ellipsis?: boolean;
        /** Lets the pointer select this text, and ctrl+C copy what it selected. */
        selectable?: boolean;
        /** Keeps text to one line, cutting off what doesn't fit. */
        noWrap?: boolean;
        /** Wraps text onto no more than this many lines, with `...` at the end of the last if there's more, as CSS's `line-clamp` does. */
        maxLines?: number;
        /** How soft a shadow is, in pixels, which is what gives something one. */
        shadowBlur?: number;
        shadowColor?: Color;
        shadowX?: number;
        shadowY?: number;
    }

    export interface ElementProps extends Style {
        key?: string | number;
        /**
         * What this is, to agents and screen readers. With text inside it, the text is what shows. With none, this is
         * what shows, if it's the kind of thing that shows text.
         */
        name?: string;
        style?: Style;
        tooltip?: string;
        /** Taking hold of this moves a window with no frame. Anything inside it that takes input still gets it. */
        dragsWindow?: boolean;
        /** Takes no input. */
        inert?: boolean;
        /** Can't be used for now: it can't be clicked or have the keyboard, and it fades unless it has an `opacity`. */
        disabled?: boolean;
        children?: Child;
        /** It's given what code written for the web looks for, like `preventDefault()`, which has nothing behind it here. */
        onClick?: (event: ClickEvent) => void;
        onFocus?: () => void;
        onToggle?: () => void;
        /** An input's new text. */
        onChange?: (value: string) => void;
        onScroll?: (position: number) => void;
        /** A key that's been pressed here or inside here, like `'enter'` or `'ctrl+s'`. It goes on to what's around this, unless this returns `true`. */
        onKeyPress?: (key: string) => unknown;
        onDismiss?: () => void;
        /** After the second click of a double click, which also counts as a click. */
        onDoubleClick?: () => void;
        /** The other mouse button was clicked here, at this position in the window, which is where a menu would go. */
        onContextMenu?: (x: number, y: number) => void;
        /**
         * The main button went down here, and the pointer's moving. `phase` is `start`, `move`, `end` or `cancel`, `x` and `y` are
         * from the top left of this, and `dx` and `dy` are how far it's come since the last time.
         */
        onDrag?: (phase: 'start' | 'move' | 'end' | 'cancel', x: number, y: number, dx: number, dy: number) => void;
        /** On the `window`, its close button was clicked. With this, it stays open until `mainWindow.close()` is called. */
        onClose?: () => void;
        /** Files dropped on this. On the `window`, that also tells the system that it takes them. */
        onDropFiles?: (paths: string[]) => void;
        // Events as the web has them, which are as they are on a page's elements: see `PageProps`.
        /** Every click on this or anything in it, before what was clicked hears it, whether that lets it go on or not. */
        onClickCapture?: (event: ClickEvent) => void;
        onBlur?: (event: PageFocusEvent) => void;
        onMouseEnter?: (event: PageMouseEvent) => void;
        onMouseLeave?: (event: PageMouseEvent) => void;
        onMouseOver?: (event: PageMouseEvent) => void;
        onMouseOut?: (event: PageMouseEvent) => void;
        onPointerEnter?: (event: PagePointerEvent) => void;
        onPointerLeave?: (event: PagePointerEvent) => void;
        onPointerOver?: (event: PagePointerEvent) => void;
        onPointerOut?: (event: PagePointerEvent) => void;
        onPointerDown?: (event: PagePointerEvent) => void;
        onPointerDownCapture?: (event: PagePointerEvent) => void;
        onPointerUp?: (event: PagePointerEvent) => void;
        onPointerUpCapture?: (event: PagePointerEvent) => void;
        /** At most once a frame, and while the main button's down on this, wherever the pointer goes. */
        onPointerMove?: (event: PagePointerEvent) => void;
        onPointerMoveCapture?: (event: PagePointerEvent) => void;
        onMouseDown?: (event: PageMouseEvent) => void;
        onMouseDownCapture?: (event: PageMouseEvent) => void;
        onMouseUp?: (event: PageMouseEvent) => void;
        onMouseUpCapture?: (event: PageMouseEvent) => void;
        onMouseMove?: (event: PageMouseEvent) => void;
        onMouseMoveCapture?: (event: PageMouseEvent) => void;
        /** `preventDefault()` keeps this from scrolling. */
        onWheel?: (event: PageWheelEvent) => void;
        onWheelCapture?: (event: PageWheelEvent) => void;
        /** Before what has the keyboard does anything with it, so `preventDefault()` keeps an input from typing a letter. `onKeyPress` comes after. */
        onKeyDown?: (event: PageKeyboardEvent) => void;
        onKeyDownCapture?: (event: PageKeyboardEvent) => void;
        onKeyUp?: (event: PageKeyboardEvent) => void;
        onKeyUpCapture?: (event: PageKeyboardEvent) => void;
        /** A click with the middle button, or back or forward. */
        onAuxClick?: (event: PageMouseEvent) => void;
        onPaste?: (event: PageClipboardEvent) => void;
        onCopy?: (event: PageClipboardEvent) => void;
        onCut?: (event: PageClipboardEvent) => void;
        onCompositionStart?: (event: PageCompositionEvent) => void;
        onCompositionUpdate?: (event: PageCompositionEvent) => void;
        onCompositionEnd?: (event: PageCompositionEvent) => void;
        /** It can be dragged as a page's can, with `onDragStart` and the rest. `onDrag` is rsui's own, and isn't that. */
        draggable?: boolean;
        onDragStart?: (event: PageDragEvent) => void;
        onDragEnd?: (event: PageDragEvent) => void;
        /** `preventDefault()` here or in `onDragOver` says what's dragged can be dropped here, from inside the app or another program. */
        onDragEnter?: (event: PageDragEvent) => void;
        onDragOver?: (event: PageDragEvent) => void;
        onDragLeave?: (event: PageDragEvent) => void;
        onDrop?: (event: PageDragEvent) => void;
    }

    // The system's own popups (plan 059, part 2).

    /** A time of day, on a 24-hour clock. */
    export interface TimeOfDay {
        /** From 0 to 23. */
        hours: number;
        /** From 0 to 59. */
        minutes: number;
    }

    /**
     * The system's own popups, each one call and a promise of its answer, the same on every system. On Android they're
     * Android's: an AlertDialog, a list dialog for a sheet, the date and time pickers, the share chooser and a Toast. On
     * Windows a confirm, alert or sheet is a task dialog, a picker is a dialog around Windows' own calendar or time
     * control, `share` is the system's share sheet, and a toast is a small window of rsui's that fades away. Elsewhere
     * they aren't there yet, and each promise is rejected saying so. A button that isn't named says what the system's own
     * does, in its language. While an app's checked with `--do` none comes up: what's asked is said, and the step
     * `dialog "ANSWER"` (or `dialog cancel`) says what the next one's answered with.
     */
    export const native: {
        /** Whether to go ahead: true once the `confirm` button's pressed, false if it's cancelled however that's done. `destructive` is for what can't be undone. */
        confirm(options: { title: string; message?: string; confirm?: string; cancel?: string; destructive?: boolean }): Promise<boolean>;
        /** Says something, and is done once it's been seen. */
        alert(options: { title: string; message?: string; ok?: string }): Promise<void>;
        /** One of these, by its place in the list, or null if none was chosen. On Windows the cancel button is always the system's own. */
        sheet(options: { title?: string; options: string[]; cancel?: string }): Promise<number | null>;
        /** A day, from `min` to `max`: a Date on it, at `value`'s time of day or midnight, or null if none was picked. */
        pickDate(options?: { value?: Date; min?: Date; max?: Date }): Promise<Date | null>;
        /** A time of day, or null if none was picked. */
        pickTime(options?: { value?: TimeOfDay }): Promise<TimeOfDay | null>;
        /** The system's share sheet with this text and address. It's done once that's up; where it's shared to isn't said. */
        share(options: { text?: string; url?: string; title?: string }): Promise<void>;
        /** A little message that goes away by itself, after a while longer with `long`. */
        toast(text: string, options?: { long?: boolean }): void;
    };
}

// What code written for a page uses: its tags as elements, CSS for the whole page, and the window as `body` has it. styled-components,
// emotion and the like are their own packages, which work here as they are.
declare module 'rsui/web' {
    /** The page's own elements, as `<dom.div>` and `<dom.button>`, for a file without React's JSX, where `<div>` is rsui's own. */
    export const dom: { [tag: string]: any };
    /** Adds some CSS to the whole page, as what's in a `.css` file is. */
    export function addCss(text: string): void;
    /** What CSS says a shape should look like, here in what it's in. */
    export function useShapeLook(className?: string): object;
    /** What CSS says about the part of a shape with this id, for going on that part: a function from an id to props. */
    export function useShapeParts(): (id: string) => object;
    /** The window, as what CSS says about `body` has it. */
    export function PageWindow(props: any): any;
    export function Input(props: any): any;
    export function TextArea(props: any): any;
    export function Select(props: any): any;
    export function Option(props: any): any;
}

/**
 * An app's `rsui.config.ts`, beside its `package.json`: who the app is and how it's packed. It's read when the app's packed,
 * and by `rsui-tsx config`, never while a packed app runs. What a window is like (`title`, `size`, `frameless`) stays as
 * exports of the main file. Whatever's left out comes from `package.json` or the folder's name.
 *
 * ```ts
 * import { defineConfig } from 'rsui/config';
 * export default defineConfig(({ platform }) => ({
 *     id: 'com.example.todo',
 *     name: 'To-do',
 *     icon: 'icon.png',
 *     permissions: platform === 'android' ? ['network'] : [],
 * }));
 * ```
 */
declare module 'rsui/config' {
    export type Platform = 'windows' | 'android' | 'macos' | 'linux';
    /** What an app may ask the system for. Android is told in its manifest; elsewhere it's asked for when it's first used. */
    export type Permission = 'network' | 'notifications' | 'camera' | 'microphone' | 'location' | 'vibrate';
    export interface AppConfig {
        /**
         * Who the app is, written the other way round from a web address, as `com.example.todo`: Android's package name,
         * Apple's bundle id, the app id Linux and Windows know it by. Two apps with the same one replace each other.
         * Otherwise `dev.rsui.<its name>`.
         */
        id?: string;
        /** What the launcher, the taskbar and the title of its store page show. Otherwise `package.json`'s name. */
        name?: string;
        /** As `1.2.3`. Otherwise `package.json`'s. */
        version?: string;
        /** A .png (256 by 256 is best) or a .ico, from this file's folder. */
        icon?: string;
        /** Files to pack beside what the app loads, as `assets/**`, and what to leave out of them, as `**\/*.psd`. */
        files?: { include?: string[]; exclude?: string[] };
        permissions?: Permission[];
        /**
         * What rsui's own elements with `native` look like: `system` is the platform's own (Material on Android, Fluent on
         * Windows, macOS's on a Mac); the others are that one everywhere.
         */
        theme?: 'system' | 'material' | 'fluent' | 'mac';
        android?: {
            /** 29 (Android 10) or later. */
            minSdk?: number;
            targetSdk?: number;
            orientation?: 'any' | 'portrait' | 'landscape';
            /** Otherwise made from `version`: 1.2.3 is 10203. Each one installed has to be bigger than the one before. */
            versionCode?: number;
        };
        macos?: { category?: string; minimumSystemVersion?: string };
        windows?: {};
        linux?: { categories?: string[] };
    }
    export interface ConfigContext {
        /** Which system it's being packed for. */
        platform: Platform;
        mode: 'development' | 'production';
    }
    export function defineConfig(config: AppConfig | ((context: ConfigContext) => AppConfig)): typeof config;
}

/**
 * What `<webview>` says and is given: a web page from elsewhere, in the system's browser view (WebView2). It's a window of its
 * own over the app's, cut to what shows of its element, its corners and to what's drawn on top of it, such as a menu. Under
 * something see-through, as a dimmed backdrop, it shows a still picture of the page instead, which that's drawn over. Nothing of
 * it is loaded until an app has one. With `--do` it's never started: a load comes back as one that worked, and the `webview`
 * steps make the page do the rest.
 */
declare module 'rsui/webview' {
    /** What a view says about itself, each time any of it changes. */
    export interface WebViewState {
        url: string;
        title: string;
        loading: boolean;
        canGoBack: boolean;
        canGoForward: boolean;
        /** The page's icon, drawn 64 pixels square, as a PNG `data:` address, once it's found, or ''. */
        icon: string;
        /** Why the last page didn't load, with Chromium's error number and name, such as -105 and ERR_NAME_NOT_RESOLVED. */
        error: { url: string; code: number; description: string } | null;
        /** What to do about there being no WebView2 on this computer, if there's none. */
        unavailable: string | null;
        /** Whether something in the page, such as a video, is full screen. */
        fullScreen: boolean;
    }
    /** A key the app keeps for itself while a page has the keyboard, by the name it was given in `keys`. */
    export interface WebViewKey {
        name: string;
        code: number;
        ctrl: boolean;
        shift: boolean;
        alt: boolean;
    }
    /** What's given back through a `<WebView>`'s `ref`. */
    export interface WebViewHandle {
        readonly state: WebViewState;
        go(url: string): void;
        back(): void;
        forward(): void;
        reload(): void;
        stop(): void;
        /** Gives the page the keyboard. */
        focus(): void;
        /** Gives the keyboard back to the app's window, if the page has it. */
        blur(): void;
        devTools(): void;
        zoom(factor: number): void;
        /** Sends the page a message, which it hears with `chrome.webview.addEventListener('message')`. */
        post(value: unknown): void;
        /** Runs a script in the page, and gives what it comes to. */
        run(script: string): Promise<any>;
        /** Forgets the cookies of the view's profile. */
        clearCookies(): void;
    }
    export interface WebViewProps {
        /** Where it goes. Changing it goes there; the page going somewhere by itself doesn't change it. */
        src?: string;
        /** Cookies and storage of their own, kept in the app's folder, for every view with this name. */
        profile?: string;
        /** Nothing it keeps lasts past it. */
        private?: boolean;
        userAgent?: string;
        /** Keys the app keeps while a page has the keyboard, such as `'Ctrl+T'` or `'F6'`, which reach `onKey` instead of the page. */
        keys?: string[];
        zoom?: number;
        /** In a page. */
        className?: string;
        style?: any;
        ref?: { current: WebViewHandle | null } | ((view: WebViewHandle | null) => void);
        /** What shows until the page starts arriving, and instead of it if it can't be shown. */
        children?: any;
        onState?: (state: WebViewState) => void;
        /** The page asked for another window, as a link with `target=_blank` does. It never gets one by itself. */
        onNewWindow?: (request: { url: string; user: boolean }) => void;
        onKey?: (key: WebViewKey) => void;
        onMessage?: (value: any) => void;
        onFocus?: (focused: boolean) => void;
        /** The page asked to be closed, with `window.close()`. */
        onClose?: () => void;
        onUnavailable?: (why: string) => void;
    }
    /** The version of WebView2 on this computer, or '' if there's none. */
    export function runtime(): string;
}

// Sockets and threads.
declare module 'net' {
    export class Socket {
        constructor(options?: { allowHalfOpen?: boolean });
        connecting: boolean;
        readonly readyState: 'opening' | 'open' | 'readOnly' | 'writeOnly' | 'closed';
        remoteAddress?: string;
        remotePort?: number;
        localAddress?: string;
        localPort?: number;
        bytesRead: number;
        bytesWritten: number;
        connect(port: number, host?: string, done?: () => void): this;
        connect(path: string, done?: () => void): this;
        connect(options: { port?: number; host?: string; path?: string }, done?: () => void): this;
        setEncoding(encoding: string): this;
        setTimeout(wait: number, done?: () => void): this;
        setNoDelay(on?: boolean): this;
        setKeepAlive(on?: boolean): this;
        write(data: string | Uint8Array, done?: () => void): boolean;
        end(data?: string | Uint8Array, done?: () => void): this;
        destroy(error?: Error): this;
        on(event: 'data', listener: (data: any) => void): this;
        on(event: 'error', listener: (error: Error & { code?: string }) => void): this;
        on(event: 'close', listener: (failed: boolean) => void): this;
        on(event: string, listener: (...given: any[]) => void): this;
        once(event: string, listener: (...given: any[]) => void): this;
        off(event: string, listener: (...given: any[]) => void): this;
    }
    export class Server {
        readonly listening: boolean;
        /** With no host it can be reached from other machines, which the firewall asks someone about. Give `'127.0.0.1'` for only this one. */
        listen(port?: number, host?: string, done?: () => void): this;
        listen(port?: number, done?: () => void): this;
        /** A named pipe, as `\\\\.\\pipe\\name`, which only this machine can reach. */
        listen(path: string, done?: () => void): this;
        listen(options: { port?: number; host?: string; path?: string }, done?: () => void): this;
        close(done?: (error?: Error) => void): this;
        address(): { address: string; port: number; family: string } | string | null;
        on(event: 'connection', listener: (socket: Socket) => void): this;
        on(event: 'error', listener: (error: Error & { code?: string }) => void): this;
        on(event: string, listener: (...given: any[]) => void): this;
    }
    export function createServer(onConnection?: (socket: Socket) => void): Server;
    export function connect(port: number, host?: string, done?: () => void): Socket;
    export function connect(path: string, done?: () => void): Socket;
    export function connect(options: { port?: number; host?: string; path?: string }, done?: () => void): Socket;
    export const createConnection: typeof connect;
    export function isIP(text: string): 0 | 4 | 6;
    export function isIPv4(text: string): boolean;
    export function isIPv6(text: string): boolean;
}
declare module 'dns' {
    export function lookup(host: string, done: (error: Error | null, address: string, family: number) => void): void;
    export const promises: { lookup(host: string, options?: { all?: boolean; family?: number }): Promise<any> };
}
declare module 'dgram' {
    export function createSocket(type: 'udp4' | 'udp6', listener?: (message: Uint8Array, from: { address: string; port: number; size: number }) => void): {
        bind(port?: number, address?: string, done?: () => void): void;
        send(message: string | Uint8Array, port: number, address?: string, done?: (error: Error | null, length: number) => void): void;
        address(): { address: string; port: number; family: string } | null;
        close(done?: () => void): void;
        on(event: string, listener: (...given: any[]) => void): void;
    };
}
declare module 'worker_threads' {
    export const isMainThread: boolean;
    export const threadId: number;
    export const workerData: any;
    export const parentPort: null | {
        postMessage(message: unknown): void;
        close(): void;
        on(event: 'message', listener: (message: any) => void): void;
    };
    export class Worker {
        /** What's given goes across as a copy. A relative path is from the app's main file. */
        constructor(file: string | URL, options?: { workerData?: unknown });
        readonly threadId: number;
        postMessage(message: unknown): void;
        /** It stops when it's between things, as nothing can stop code that's running. */
        terminate(): Promise<number>;
        on(event: 'message', listener: (message: any) => void): this;
        on(event: 'error', listener: (error: Error) => void): this;
        on(event: 'exit', listener: (code: number) => void): this;
        on(event: 'online', listener: () => void): this;
    }
}
/** A thread with code in it, as the web has it. Inside it, `self.onmessage` and `postMessage` are the other side. */
declare class Worker {
    constructor(file: string | URL, options?: { name?: string });
    onmessage: ((event: { data: any }) => void) | null;
    onerror: ((event: { message: string; error: Error }) => void) | null;
    postMessage(message: unknown): void;
    terminate(): void;
    addEventListener(kind: string, listener: (event: any) => void): void;
    removeEventListener(kind: string, listener: (event: any) => void): void;
}
declare class WebSocket {
    static readonly CONNECTING: 0;
    static readonly OPEN: 1;
    static readonly CLOSING: 2;
    static readonly CLOSED: 3;
    constructor(url: string | URL, protocols?: string | string[]);
    readonly url: string;
    readyState: number;
    protocol: string;
    binaryType: 'blob' | 'arraybuffer';
    onopen: ((event: any) => void) | null;
    onmessage: ((event: { data: any }) => void) | null;
    onerror: ((event: any) => void) | null;
    onclose: ((event: { code: number; reason: string; wasClean: boolean }) => void) | null;
    send(data: string | ArrayBuffer | ArrayBufferView): void;
    close(code?: number, reason?: string): void;
    addEventListener(kind: string, listener: (event: any) => void): void;
    removeEventListener(kind: string, listener: (event: any) => void): void;
}

// What a program can ask of the desktop, apart from what's on its windows, as one object for each part. Each is only loaded the
// first time it's used. With `--do` and `--agent` none of it happens, and it's said instead. With `RSUI_SANDBOX` set to a folder,
// what would last in the user's settings or credentials is kept there instead.
declare module 'rsui/desktop' {
    export interface MenuItem {
        label?: string;
        /** Keys that do the same, like `ctrl+o`, which work in a menu bar. A tray's menu just shows them. */
        shortcut?: string;
        disabled?: boolean;
        checked?: boolean;
        separator?: boolean;
        /** What opens from this. */
        items?: MenuItem[];
        onClick?: () => void;
    }
    export interface DialogOptions {
        title?: string;
        /** A file or folder to start from. */
        start?: string;
        /** What kinds of file to show, as in `{ name: 'Pictures', extensions: ['png', 'jpg'] }`. */
        filters?: { name: string; extensions: string[] }[];
    }
    export type MessageAnswer = 'ok' | 'cancel' | 'yes' | 'no' | 'retry';
    export interface MessageOptions {
        title?: string;
        message?: string;
        kind?: 'info' | 'warning' | 'error' | 'question';
        buttons?: 'ok' | 'ok-cancel' | 'yes-no' | 'yes-no-cancel' | 'retry-cancel';
    }
    export interface Display {
        x: number;
        y: number;
        width: number;
        height: number;
        workX: number;
        workY: number;
        workWidth: number;
        workHeight: number;
        scale: number;
        primary: boolean;
    }
    /** An icon: an `.ico` file or a picture the app has, or one's bytes. */
    export type Icon = string | Uint8Array;
    /** A window, by its id or its controls, or the main one. */
    export type WhichWindow = number | { id: number };
    /** Opening things with the programs the system has for them. */
    export const shell: {
        /** Opens a web page in the browser, an address in the mail program, or a file or folder with what the system has for it. Only http, https and mailto addresses can be opened. */
        open(target: string): Promise<void>;
        /** Shows a file in its folder in Explorer, picked out. */
        reveal(path: string): Promise<void>;
        /** Puts files in the Recycle Bin, from where they can be brought back. */
        trash(paths: string | string[]): Promise<void>;
        /** Asks which program to open a file with. */
        openWith(path: string): Promise<void>;
        /** Shows the system's properties of a file or folder. */
        properties(path: string): Promise<void>;
    };
    export interface ClipboardContents {
        text?: string;
        html?: string;
        files?: string[];
        /** A picture, as a PNG file's bytes. */
        image?: Uint8Array;
    }
    /** The system's clipboard. */
    export const clipboard: {
        read(): Promise<ClipboardContents>;
        /** Puts this in place of what's there, each that's given. */
        write(contents: ClipboardContents | string): Promise<void>;
        clear(): Promise<void>;
        /** What's on it now: `text/plain`, `text/html`, `image/png`, `files`, and others by their names. */
        formats(): string[];
        /** Runs whenever something's put on it, by this program or another. What it gives back stops it. */
        onChange(run: () => void): () => void;
    };
    /** Text kept in Windows' Credential Manager, where only this user can read it. */
    export const secrets: {
        get(key: string): Promise<string | undefined>;
        /** Up to 2560 bytes. */
        set(key: string, value: string): Promise<void>;
        /** It's whether there was anything. */
        delete(key: string): Promise<boolean>;
    };
    /** The system's own dialogs. The window keeps drawing while they're up. */
    export const dialogs: {
        /** Asks someone to choose a file, or a folder with `folders: true`. It's `null` if they don't. */
        open(options?: DialogOptions & { folders?: boolean }): Promise<string | null>;
        /** Asks someone where to save something, and whether to replace what's there. It's `null` if they don't say. */
        save(options?: DialogOptions): Promise<string | null>;
        /** Asks with the system's message box. */
        message(options: MessageOptions): Promise<MessageAnswer>;
        /** Asks for a color, as `#rrggbb`. It's `null` if they don't choose one. `dialog "#rrggbb"` answers it in steps. */
        pickColor(options?: { initial?: string }): Promise<string | null>;
        /** Asks for a font. It's `null` if they don't choose one. `dialog` answers it in steps, with JSON of a `FontChoice`. */
        pickFont(options?: { initial?: Partial<FontChoice> }): Promise<FontChoice | null>;
    };
    export interface FontChoice {
        family: string;
        /** In points. */
        size: number;
        /** From 100 to 900, with 400 as usual and 700 bold. */
        weight: number;
        italic: boolean;
        /** As `#rrggbb`. */
        color: string;
    }
    /** The system's notifications. One shows at a time, from the tray, so showing another takes the last one's place. */
    export const notifications: {
        /** Shows one. It's its id. */
        show(notification: { title?: string; body?: string; onClick?: () => void }): number;
        /** Changes what it says, if it's still showing. It's whether it was. */
        update(id: number, notification: { title?: string; body?: string }): boolean;
        dismiss(id: number): void;
        dismissAll(): void;
        /** Runs when any is clicked. What it gives back stops it. */
        onClick(run: (clicked: { id: number }) => void): () => void;
        /** Runs when one of a notification's buttons is pressed. Notifications from the tray have none, so it isn't told yet. */
        onAction(run: (pressed: { id: number; action: string }) => void): () => void;
    };
    export const tray: {
        /** Puts an icon in the tray, or changes the one that's there. Clicking on it with the other button opens its menu. */
        set(tray: { tooltip?: string; /** It's the program's own icon otherwise. */ icon?: Icon; onClick?: () => void; menu?: MenuItem[] }): void;
        remove(): void;
        /** Where the icon is on the screen, in physical pixels, or `null`. */
        getBounds(): { x: number; y: number; width: number; height: number } | null;
    };
    /** A window's button on the taskbar. */
    export const taskbar: {
        /** How far along something is, from 0 to 1. `none` takes it off. */
        setProgress(value: number, state?: 'normal' | 'paused' | 'error' | 'indeterminate' | 'none', window?: WhichWindow): void;
        /** A small picture over the button, as a count or a status, with words for screen readers, or `null` to take it off. */
        setOverlayIcon(icon: Icon | null, description?: string, window?: WhichWindow): void;
        /** Has the button flash: until the user comes to it, or a few times with `critical: false`, or stop with `stop: true`. */
        requestAttention(options?: { critical?: boolean; stop?: boolean }, window?: WhichWindow): void;
        /**
         * What right-clicking the program's button shows, in place of what it put there before: `tasks` under Tasks, and each
         * of `categories` under its name. Choosing one starts this program with `args` after it.
         */
        setJumpList(list: { tasks?: JumpItem[]; categories?: { name: string; items: JumpItem[] }[] }): void;
        clearJumpList(): void;
    };
    export interface JumpItem {
        title: string;
        /** What the program's started with, after it, when this is chosen. */
        args?: string;
        /** An `.ico` file, or the program's own icon otherwise. */
        icon?: string;
        /** What's said when the pointer's over it. */
        description?: string;
    }
    export const shortcuts: {
        /** Runs this when these keys are pressed, like `ctrl+shift+k`, whatever's being used at the time. It throws if something else has them. What it gives back stops it. */
        register(keys: string, run: () => void): () => void;
        /** `register`, for as long as this component's there. */
        useShortcut(keys: string, run: () => void): void;
    };
    export interface Playback {
        readonly file: string;
        readonly playing: boolean;
        volume: number;
        muted: boolean;
        loop: boolean;
        /** How long it is, in milliseconds, once that's known. */
        readonly duration: number | null;
        /** Where it's up to, in milliseconds. */
        readonly position: number;
        seek(milliseconds: number): void;
        play(): Playback;
        pause(): void;
        stop(): void;
    }
    export const sound: {
        /** Plays an MP3 or WAV file. */
        play(file: string, options?: { volume?: number; loop?: boolean; onEnded?: () => void }): Playback;
        /** One of the sounds the system makes. */
        beep(kind?: 'info' | 'warning' | 'error' | 'question'): void;
    };
    export interface AppPaths {
        home?: string;
        appData?: string;
        localAppData?: string;
        /** This program's own folder in `appData`. */
        userData?: string;
        temp?: string;
        desktop?: string;
        documents?: string;
        downloads?: string;
        music?: string;
        pictures?: string;
        videos?: string;
        exe?: string;
        /** The app's main file. */
        app?: string;
    }
    export const app: {
        /** What this program is, from its `package.json` if it has one. */
        getInfo(): { name: string; version: string; packed: boolean; path?: string };
        getPaths(): AppPaths;
        /** Runs before `quit()` ends the program, which `preventDefault()` stops. What it gives back stops it. */
        onBeforeQuit(run: (event: { preventDefault(): void }) => void): () => void;
        /** Runs with each link the program's asked to open, at the start and from another start of it once it's the only one running. See `protocols`. */
        onOpenUrl(run: (url: string) => void): () => void;
        /** The same, for files. See `fileAssociations`. */
        onOpenFiles(run: (paths: string[]) => void): () => void;
        /** Says these were opened, for the system's lists of what's been used lately. */
        setRecentDocuments(paths: string | string[]): void;
        clearRecentDocuments(): void;
        /** Says whether this program starts when someone logs in. */
        setStartAtLogin(on?: boolean, options?: { args?: string[] }): void;
        startsAtLogin(): boolean;
        /** Says whether this is the only one of this program running. If it isn't, it should end, and what it was started with has gone to the one that's running, to `onSecond`. */
        singleInstance(onSecond?: (args: string[]) => void): boolean;
        /** Starts this program again, and closes this one. */
        relaunch(): void;
        /** Closes the main window, which ends the program, unless `onBeforeQuit` says not to. It's whether it's ending. */
        quit(): boolean;
    };
    /** Links that start a scheme of the program's own, like `myapp:`, which start it. */
    export const protocols: {
        register(scheme: string): void;
        unregister(scheme: string): void;
        isRegistered(scheme: string): boolean;
    };
    /** Kinds of file this program can open. */
    export const fileAssociations: {
        /** It's one of what "Open with" offers for them. Only the user can choose what opens them unasked. */
        register(kind: { extension: string; description?: string; icon?: string }): void;
        unregister(extension: string): void;
        openDefaultAppsSettings(): Promise<void>;
    };
    export const displays: {
        /** The screens there are, in physical pixels. */
        list(): Display[];
        /** Where the pointer is, in physical pixels. */
        pointer(): [number, number];
        primary(): Display | null;
        /** The screen a point is on, in physical pixels, or the nearest. */
        forPoint(x: number, y: number): Display | null;
        /** The screen most of a window is on, by its id, or the main window's. */
        forWindow(windowId?: number): Display | null;
        /** Runs when a screen's added, taken away or changed. What it gives back stops it. */
        onChange(run: (displays: Display[]) => void): () => void;
    };
    export interface Accessibility {
        reducedMotion: boolean;
        highContrast: boolean;
        /** How much bigger text is wanted. */
        textScale: number;
    }
    /** What the system says about itself. */
    export const system: {
        /** Like `en-US`. */
        locale(): string;
        /** Like `Europe/Amsterdam`. */
        timeZone(): string;
        /** Since the keyboard or the mouse was last used, in milliseconds. */
        idleTime(): number;
        /** The color chosen for Windows, as `#rrggbb`. */
        accentColor(): string | null;
        accessibility(): Accessibility;
        /** Runs when any of these change. What it gives back stops it. */
        onChange(run: (now: { locale: string; accentColor: string | null; accessibility: Accessibility }) => void): () => void;
    };
    export const power: {
        /** Stops the machine going to sleep and the screen going off, until it's turned off. */
        keepAwake(on?: boolean): void;
        /** `level` is from 0 to 1, or `null` with no battery. */
        battery(): { onBattery: boolean; level: number | null; charging: boolean };
        /** Runs when the machine's about to sleep, or has woken up, or its power has changed. What it gives back stops it. */
        onChange(run: (what: 'suspend' | 'resume' | 'power-changed') => void): () => void;
    };
    export interface NetworkStatus {
        online: boolean;
        /** Only this network can be reached, or a sign-in page is in the way. */
        localOnly: boolean;
        /** It costs by how much is used. */
        metered: boolean;
    }
    export const network: {
        status(): NetworkStatus;
        onChange(run: (status: NetworkStatus) => void): () => void;
    };
    export const drag: {
        /** Starts dragging these files out of the program, to drop in another, while a button's held. It's what was done with them. */
        startFiles(paths: string | string[]): Promise<'copy' | 'move' | 'link' | 'none'>;
    };
    export const menus: {
        /** Shows a menu where the pointer is, or at a point on the screen in physical pixels. `popup ITEM` chooses from it in steps. */
        popup(items: MenuItem[], at?: { x: number; y: number }): void;
    };
    /** A picture: `png` is a PNG file's bytes, and `url` a `data:` URL of it, for an `<image>` or an `<img>`. */
    export interface Picture {
        width: number;
        height: number;
        png: Uint8Array;
        url: string;
    }
    export interface FolderEntry {
        name: string;
        folder: boolean;
        hidden: boolean;
        system: boolean;
        readonly: boolean;
        /** A symbolic link or a junction. */
        link: boolean;
        size: number;
        modified: number;
        created: number;
    }
    export interface Drive {
        /** Like `C:\\`. */
        path: string;
        label: string;
        kind: 'fixed' | 'removable' | 'network' | 'optical' | 'ram' | 'unknown';
        /** In bytes, 0 when it can't be read. */
        total: number;
        free: number;
    }
    /** What Explorer shows for files. These are only read, so they're real while an app's being checked too. */
    export const files: {
        /** What's in a folder, read in one pass on another thread, with each thing's size, times (in milliseconds) and attributes, in the order Explorer sorts names: without regard to case, and numbers by their value. */
        list(folder: string): Promise<FolderEntry[]>;
        /** The drives there are. A network drive that isn't connected can take a while to answer. */
        drives(): Promise<Drive[]>;
        /** A picture of what's in it, as a photo's or a document's first page, at most `size` pixels across. It's `null` for a file with none. */
        thumbnail(path: string, options?: { size?: number }): Promise<Picture | null>;
        /** Its icon, at most `size` pixels across. */
        icon(path: string, options?: { size?: number }): Promise<Picture>;
    };
    /** Pictures of the screen, in physical pixels. While an app's being checked, they're said, and blank. */
    export const capture: {
        /** A whole screen, by where it is in `displays.list()`, or the main one. */
        screen(displayIndex?: number): Promise<Picture>;
        /** A rectangle, from the main screen's top left. */
        region(rectangle: { x?: number; y?: number; width: number; height: number }): Promise<Picture>;
    };
    export interface AudioDevice {
        /** What the system knows it by. */
        id: string;
        name: string;
        kind: 'output' | 'input';
        /** Whether it's what the system uses for that. */
        isDefault: boolean;
    }
    /** The devices that play and record sound. They're only read, so they're real while an app's being checked too. */
    export const audio: {
        devices(): AudioDevice[];
        /** Runs with the devices when one's plugged in or taken out. What it gives back stops it. */
        onChange(run: (devices: AudioDevice[]) => void): () => void;
    };
    export type MediaKey = 'play' | 'pause' | 'playPause' | 'next' | 'previous' | 'stop';
    /** The keys for what's playing. */
    export const media: {
        /** Runs when one's pressed while one of this program's windows has the keyboard. `media KEY` presses one in steps. What it gives back stops it. */
        onKey(run: (key: MediaKey) => void): () => void;
    };
    export interface Store {
        get(key: string): any;
        has(key: string): boolean;
        /** Keeps this for the key. It has to be something JSON can hold. */
        set(key: string, value: unknown): void;
        delete(key: string): boolean;
        keys(): string[];
        clear(): void;
        /** Runs after each change, with `from` saying which way in it came. `key` is null for `clear()`. What it gives back stops it. */
        onChange(run: (change: { key: string | null; value: any; old: any; from: 'storage' | 'localStorage' }) => void): () => void;
    }
    /** What the app keeps between runs, in its data folder. `localStorage` is the same store, so what either sets the other reads. */
    export const storage: Store & {
        /** Another store of the app's, of its own. */
        open(name: string): Store;
    };
    /** A packed program replacing itself with a newer version, only ever with exactly the file that was meant. */
    export const updates: {
        /** Asks a JSON file of `{ version, url, sha256, notes }` whether there's a newer version than this one. */
        check(url: string): Promise<{ available: boolean; version: string; url: string; sha256: string; notes: string }>;
        /** Downloads it, and checks it's the one meant. It's the downloaded file. */
        download(update: { url: string; sha256: string; onProgress?: (progress: { received: number; total: number | null }) => void }): Promise<string>;
        /** Replaces this program with the file, as long as it has exactly this SHA-256. It's what runs next time, or now with `app.relaunch()`. */
        install(file: string, sha256: string): void;
        /** Says this version works. A new version that's started twice without saying so is replaced with the one before it. */
        confirmHealthy(): void;
        /** Whether this is a new version that hasn't said it works yet. */
        waitingToConfirm(): boolean;
    };
}
/** A notification, as the web has it, over `notifications`. */
declare class Notification {
    static permission: 'granted';
    static requestPermission(done?: (permission: 'granted') => void): Promise<'granted'>;
    constructor(title: string, options?: { body?: string; tag?: string; data?: unknown });
    title: string;
    body: string;
    onclick: ((event: { type: 'click'; target: Notification }) => void) | null;
    onclose: ((event: { type: 'close'; target: Notification }) => void) | null;
    addEventListener(kind: string, listener: (event: any) => void): void;
    removeEventListener(kind: string, listener: (event: any) => void): void;
    close(): void;
}
/** A sound, as the web has it, over `sound`. MP3 and WAV files can both be played. */
declare class Audio {
    constructor(src?: string);
    src: string;
    loop: boolean;
    muted: boolean;
    volume: number;
    currentTime: number;
    readonly duration: number;
    paused: boolean;
    ended: boolean;
    onended: ((event: { type: 'ended'; target: Audio }) => void) | null;
    play(): Promise<void>;
    pause(): void;
    load(): void;
    addEventListener(kind: string, listener: (event: any) => void): void;
    removeEventListener(kind: string, listener: (event: any) => void): void;
}
/** The system's message box, with everything waiting for it, as a page's is. */
declare function alert(message?: unknown): void;
declare function confirm(message?: unknown): boolean;
/** The main screen, as the web has it, in logical pixels. */
declare const screen: { readonly width: number; readonly height: number; readonly availWidth: number; readonly availHeight: number; readonly availLeft: number; readonly availTop: number; colorDepth: number; pixelDepth: number };

// What code written for React imports. It's the same hooks and elements as 'rsui', with what React has on top of them.
declare module 'react' {
    import { Child, Component as FunctionComponent, Context, Element } from 'rsui';
    export * from 'rsui';
    export type Ref<T> = { current: T | null } | ((value: T | null) => void) | null;
    export type ComponentType<P = {}> = FunctionComponent<P> | (new (props: P) => Component<P, any>);
    export interface ErrorInfo {
        componentStack: string;
    }
    export const version: string;
    /** Components already skip rendering when their props are the same, so this gives back what it's given. */
    export function memo<T>(component: T, same?: (before: any, after: any) => boolean): T;
    export function forwardRef<T, P = {}>(render: (props: P, ref: Ref<T>) => Child): FunctionComponent<P & { ref?: Ref<T>; key?: unknown }>;
    export function createRef<T>(): { current: T | null };
    export function useImperativeHandle<T>(ref: Ref<T> | undefined, make: () => T, deps?: readonly unknown[]): void;
    export function useSyncExternalStore<T>(subscribe: (changed: () => void) => () => void, getSnapshot: () => T, getServerSnapshot?: () => T): T;
    export function useId(): string;
    export function startTransition(run: () => void): void;
    export function useTransition(): [boolean, (run: () => void) => void];
    export function useDeferredValue<T>(value: T): T;
    export function useDebugValue(value: unknown, format?: (value: any) => unknown): void;
    export const useInsertionEffect: typeof import('rsui').useEffect;
    export function createContext<T>(fallback: T): Context<T> & { Consumer: FunctionComponent<{ children: (value: T) => Child }> };
    export function isValidElement(value: unknown): value is Element;
    export function createElement(type: unknown, props?: Record<string, any> | null, ...children: Child[]): Element;
    export function cloneElement(element: Element, props?: Record<string, any> | null, ...children: Child[]): Element;
    export const Children: {
        toArray(children: Child): Exclude<Child, boolean | null | undefined | Child[]>[];
        count(children: Child): number;
        map<T>(children: Child, run: (child: any, index: number) => T, self?: unknown): T[];
        forEach(children: Child, run: (child: any, index: number) => void, self?: unknown): void;
        only(children: Child): Element;
    };
    export class Component<P = {}, S = {}> {
        constructor(props: P, context?: any);
        props: Readonly<P> & { children?: Child };
        state: Readonly<S>;
        context: any;
        refs: Record<string, unknown>;
        setState(change: Partial<S> | ((state: S, props: P) => Partial<S> | null) | null, done?: () => void): void;
        forceUpdate(done?: () => void): void;
        render(): Child;
        componentDidMount?(): void;
        componentDidUpdate?(props: P, state: S, snapshot?: any): void;
        componentWillUnmount?(): void;
        shouldComponentUpdate?(props: P, state: S, context: any): boolean;
        getSnapshotBeforeUpdate?(props: P, state: S): any;
        componentDidCatch?(error: Error, info: ErrorInfo): void;
    }
    export class PureComponent<P = {}, S = {}> extends Component<P, S> {}
    // The events a page's elements are given, by the names React has for them.
    export type SyntheticEvent<T = HTMLElement> = PageEvent<T>;
    export type BaseSyntheticEvent<T = HTMLElement> = PageEvent<T>;
    export type UIEvent<T = HTMLElement> = PageEvent<T>;
    export type MouseEvent<T = HTMLElement> = PageMouseEvent<T>;
    export type PointerEvent<T = HTMLElement> = PagePointerEvent<T>;
    export type DragEvent<T = HTMLElement> = PageDragEvent<T>;
    export type DragEventHandler<T = HTMLElement> = EventHandler<PageDragEvent<T>>;
    export type WheelEvent<T = HTMLElement> = PageWheelEvent<T>;
    export type KeyboardEvent<T = HTMLElement> = PageKeyboardEvent<T>;
    export type FocusEvent<T = HTMLElement> = PageFocusEvent<T>;
    export type ClipboardEvent<T = HTMLElement> = PageClipboardEvent<T>;
    export type CompositionEvent<T = HTMLElement> = PageCompositionEvent<T>;
    export type TransitionEvent<T = HTMLElement> = PageTransitionEvent<T>;
    export type AnimationEvent<T = HTMLElement> = PageAnimationEvent<T>;
    export type ChangeEvent<T = HTMLInputElement> = PageChangeEvent<T>;
    export type FormEvent<T = HTMLElement> = PageChangeEvent<T>;
    export type EventHandler<E> = (event: E) => void;
    export type MouseEventHandler<T = HTMLElement> = EventHandler<PageMouseEvent<T>>;
    export type PointerEventHandler<T = HTMLElement> = EventHandler<PagePointerEvent<T>>;
    export type KeyboardEventHandler<T = HTMLElement> = EventHandler<PageKeyboardEvent<T>>;
    export type ChangeEventHandler<T = HTMLInputElement> = EventHandler<PageChangeEvent<T>>;
    export type WheelEventHandler<T = HTMLElement> = EventHandler<PageWheelEvent<T>>;
    export type FocusEventHandler<T = HTMLElement> = EventHandler<PageFocusEvent<T>>;
    export type HTMLAttributes<T = HTMLElement> = PageProps<T>;
    export type CSSProperties = PageStyle;
    /** What it loads shows nothing until it's there. */
    export function lazy<P>(load: () => Promise<{ default: unknown }>): FunctionComponent<P>;
    /** There's nothing to wait for here, so this just shows what's in it, and never `fallback`. */
    export const Suspense: FunctionComponent<{ children?: Child; fallback?: Child }>;
    export const StrictMode: FunctionComponent<{ children?: Child }>;
    export const Profiler: FunctionComponent<{ children?: Child; id?: string; onRender?: unknown }>;
    const React: typeof import('react');
    export default React;
}
declare module 'react-dom' {
    import { Child } from 'rsui';
    /** What's in it shows where it's written. Overlay props, `Window` or `Dialog` are what take something out of there. */
    export function createPortal(children: Child, container?: unknown, key?: unknown): Child;
    export function flushSync<T>(run: () => T): T;
    export function unstable_batchedUpdates<T>(run: (value?: any) => T, value?: any): T;
    export const version: string;
    const ReactDOM: typeof import('react-dom');
    export default ReactDOM;
}
declare module 'react/jsx-runtime' {
    import { Element, Fragment } from 'rsui';
    export { Fragment };
    export function jsx(type: unknown, props: Record<string, any>, key?: unknown): Element;
    export const jsxs: typeof jsx;
    export const jsxDEV: typeof jsx;
}

// What the engine has that isn't in the language itself.
declare function setTimeout(run: (...args: any[]) => void, milliseconds?: number, ...args: any[]): number;
declare function clearTimeout(id?: number): void;
declare function setInterval(run: (...args: any[]) => void, milliseconds?: number, ...args: any[]): number;
declare function clearInterval(id?: number): void;
declare function queueMicrotask(run: () => void): void;
/** Text that's still there next time. It's kept in a file for each program, or the one `RSUI_STORAGE_FILE` says. */
declare const localStorage: Storage;
/** The same, for as long as the program's running. */
declare const sessionStorage: Storage;
interface Storage {
    readonly length: number;
    key(at: number): string | null;
    getItem(key: string): string | null;
    setItem(key: string, value: string): void;
    removeItem(key: string): void;
    clear(): void;
    [name: string]: any;
}
/** A clock for measuring, in milliseconds since the program started, with fractions of one. */
declare const performance: { now(): number; readonly timeOrigin: number };
declare const console: {
    log(...values: unknown[]): void;
    info(...values: unknown[]): void;
    warn(...values: unknown[]): void;
    error(...values: unknown[]): void;
};

declare class URLSearchParams implements Iterable<[string, string]> {
    constructor(from?: string | Record<string, unknown> | Iterable<[string, string]>);
    readonly size: number;
    append(name: string, value: string): void;
    delete(name: string, value?: string): void;
    get(name: string): string | null;
    getAll(name: string): string[];
    has(name: string, value?: string): boolean;
    set(name: string, value: string): void;
    sort(): void;
    forEach(run: (value: string, name: string) => void): void;
    entries(): IterableIterator<[string, string]>;
    keys(): IterableIterator<string>;
    values(): IterableIterator<string>;
    [Symbol.iterator](): IterableIterator<[string, string]>;
    toString(): string;
}
declare class URL {
    constructor(url: string, base?: string | URL);
    static canParse(url: string, base?: string | URL): boolean;
    href: string;
    protocol: string;
    username: string;
    password: string;
    host: string;
    hostname: string;
    port: string;
    pathname: string;
    search: string;
    hash: string;
    readonly origin: string;
    readonly searchParams: URLSearchParams;
    toString(): string;
    toJSON(): string;
}
declare class Event {
    constructor(type: string, options?: { bubbles?: boolean; cancelable?: boolean; composed?: boolean });
    readonly type: string;
    readonly defaultPrevented: boolean;
    target: unknown;
    currentTarget: unknown;
    preventDefault(): void;
    stopPropagation(): void;
    stopImmediatePropagation(): void;
}
declare class EventTarget {
    addEventListener(type: string, listener: (event: Event) => void, options?: { once?: boolean }): void;
    removeEventListener(type: string, listener: (event: Event) => void): void;
    dispatchEvent(event: Event): boolean;
}
declare class AbortSignal extends EventTarget {
    static abort(reason?: unknown): AbortSignal;
    static timeout(milliseconds: number): AbortSignal;
    readonly aborted: boolean;
    readonly reason: unknown;
    onabort: ((event: Event) => void) | null;
    throwIfAborted(): void;
}
declare class AbortController {
    readonly signal: AbortSignal;
    abort(reason?: unknown): void;
}
type BufferEncoding = 'utf8' | 'utf-8' | 'hex' | 'base64' | 'base64url' | 'latin1' | 'ascii' | 'utf16le';
declare class Buffer extends Uint8Array {
    static from(value: string | ArrayLike<number> | ArrayBuffer, encoding?: string): Buffer;
    static alloc(size: number, fill?: number | string): Buffer;
    static allocUnsafe(size: number): Buffer;
    static isBuffer(value: unknown): boolean;
    static byteLength(value: string | ArrayBufferView, encoding?: string): number;
    static concat(list: Uint8Array[], total?: number): Buffer;
    toString(encoding?: string, start?: number, end?: number): string;
    equals(other: Uint8Array): boolean;
    write(text: string, offset?: number): number;
    toJSON(): { type: 'Buffer'; data: number[] };
}

declare const process: {
    env: Record<string, string | undefined>;
    cwd(): string;
    platform: string;
    pid: number;
    /** Ends the program there and then. */
    exit(code?: number): never;
};

/** Whole files of text, and folders. There's no `Buffer` here, so what's read is always text. */
declare module 'fs' {
    export function readFileSync(path: string, encoding: BufferEncoding | { encoding: BufferEncoding }): string;
    export function readFileSync(path: string): Buffer;
    export function writeFileSync(path: string, data: string | Uint8Array, encoding?: BufferEncoding): void;
    export function appendFileSync(path: string, data: string | Uint8Array, encoding?: BufferEncoding): void;
    export function copyFileSync(from: string, to: string): void;
    export function accessSync(path: string): void;
    export function realpathSync(path: string): string;
    export function lstatSync(path: string): { size: number; mtimeMs: number; mtime: Date; isFile(): boolean; isDirectory(): boolean };
    export function createReadStream(path: string, options?: BufferEncoding | { encoding?: BufferEncoding; start?: number; end?: number }): import('stream').Readable;
    export function createWriteStream(path: string, options?: { flags?: string }): import('stream').Writable;
    export function existsSync(path: string): boolean;
    /** Calls `listener` when something in the folder (or under it, with `recursive`) changes, from a thread that waits for Windows to say so. */
    export function watch(
        path: string,
        options?: { recursive?: boolean; persistent?: boolean } | ((event: 'rename' | 'change', filename: string | null) => void),
        listener?: (event: 'rename' | 'change', filename: string | null) => void,
    ): { close(): void; unref(): unknown; ref(): unknown };
    export function mkdirSync(path: string, options?: { recursive?: boolean }): void;
    export function unlinkSync(path: string): void;
    export function rmSync(path: string, options?: { recursive?: boolean; force?: boolean }): void;
    export function renameSync(from: string, to: string): void;
    export function readdirSync(path: string): string[];
    export function statSync(path: string): { size: number; mtimeMs: number; isFile(): boolean; isDirectory(): boolean };
    /** The same things on another thread, which is what to use for anything that could be large or slow. */
    export const promises: {
        readFile(path: string, encoding: BufferEncoding | { encoding: BufferEncoding }): Promise<string>;
        readFile(path: string): Promise<Buffer>;
        copyFile(from: string, to: string): Promise<void>;
        access(path: string): Promise<void>;
        writeFile(path: string, data: string): Promise<void>;
        appendFile(path: string, data: string): Promise<void>;
        mkdir(path: string, options?: { recursive?: boolean }): Promise<void>;
        unlink(path: string): Promise<void>;
        rm(path: string, options?: { recursive?: boolean; force?: boolean }): Promise<void>;
        rename(from: string, to: string): Promise<void>;
        readdir(path: string): Promise<string[]>;
        stat(path: string): Promise<{ size: number; mtimeMs: number; isFile(): boolean; isDirectory(): boolean }>;
    };
}

/** A request on another thread. What comes back is text, read whole, and there are no streams. */
declare function fetch(
    url: string,
    init?: { method?: string; headers?: Record<string, string> | [string, string][]; body?: string | URLSearchParams; signal?: AbortSignal },
): Promise<Response>;
declare class Headers implements Iterable<[string, string]> {
    constructor(from?: Record<string, string> | [string, string][] | Headers);
    get(name: string): string | null;
    has(name: string): boolean;
    set(name: string, value: string): void;
    append(name: string, value: string): void;
    delete(name: string): void;
    forEach(run: (value: string, name: string) => void): void;
    entries(): IterableIterator<[string, string]>;
    [Symbol.iterator](): IterableIterator<[string, string]>;
}
declare class Response {
    constructor(body?: string, init?: { status?: number; statusText?: string; headers?: Record<string, string> });
    readonly ok: boolean;
    readonly status: number;
    readonly statusText: string;
    readonly headers: Headers;
    readonly url: string;
    readonly bodyUsed: boolean;
    text(): Promise<string>;
    json(): Promise<any>;
    clone(): Response;
}

declare module 'path' {
    export function join(...parts: string[]): string;
    export function dirname(path: string): string;
    export function basename(path: string, extension?: string): string;
    export function extname(path: string): string;
    export function resolve(...parts: string[]): string;
}

// A page's elements, as JSX written for a page has them: `<div className>`, `onPointerMove`, `document.addEventListener` and
// the rest. See "Writing an app as for a page" in DESIGN.md.

/**
 * An element of a page, as a `ref` holds it and an event says what it happened to. What's said here is what there is. What
 * an event found that has no `ref` is made up from the tree: its tag is what it is there, as `DIV` for a box, and its
 * class, `data-` attributes and where it is are its own.
 */
interface PageElement {
    readonly tagName: string;
    readonly nodeName: string;
    className: string;
    id: string;
    readonly dataset: Record<string, string | undefined>;
    /** What it is in, as far as what an event went through says. */
    readonly parentElement: PageElement | null;
    focus(): void;
    blur(): void;
    click(): void;
    /** Whether this is `other`, or has it inside, as far as an event that found `other` says. */
    contains(other: PageElement | null | undefined | EventTarget): boolean;
    closest(selector: string): PageElement | null;
    /** A tag, `.class`, `#id`, `[data-name]` or `[data-name="value"]`, together or with commas between. */
    matches(selector: string): boolean;
    getAttribute(name: string): string | null;
    hasAttribute(name: string): boolean;
    /** Where it was when an event found it, from the window's top left. */
    getBoundingClientRect(): { x: number; y: number; left: number; top: number; right: number; bottom: number; width: number; height: number };
    readonly offsetWidth: number;
    readonly offsetHeight: number;
    readonly clientWidth: number;
    readonly clientHeight: number;
    /** Nothing to do, as what the main button went down on hears the pointer move until it comes up anyway. */
    setPointerCapture(pointerId: number): void;
    releasePointerCapture(pointerId: number): void;
    hasPointerCapture(pointerId: number): boolean;
    scrollIntoView(options?: unknown): void;
    select(): void;
    /** These hear nothing on an element: on `document` and `window` they do. */
    addEventListener(type: string, listener: (event: any) => void, options?: unknown): void;
    removeEventListener(type: string, listener: (event: any) => void, options?: unknown): void;
}
interface HTMLElement extends PageElement {}
interface HTMLDivElement extends HTMLElement {}
interface HTMLSpanElement extends HTMLElement {}
interface HTMLParagraphElement extends HTMLElement {}
interface HTMLUListElement extends HTMLElement {}
interface HTMLLIElement extends HTMLElement {}
interface HTMLAnchorElement extends HTMLElement {}
interface HTMLButtonElement extends HTMLElement {}
interface HTMLFormElement extends HTMLElement {}
interface HTMLLabelElement extends HTMLElement {}
interface HTMLCanvasElement extends HTMLElement {}
/** A field's `value` is what it shows now, and setting it changes that. */
interface HTMLInputElement extends HTMLElement {
    value: string;
    checked: boolean;
}
interface HTMLTextAreaElement extends HTMLInputElement {}
interface HTMLSelectElement extends HTMLInputElement {}
interface HTMLImageElement extends HTMLElement {
    readonly naturalWidth: number;
    readonly naturalHeight: number;
    readonly complete: boolean;
    readonly src: string;
}

/** What every event a page's element is given has. `currentTarget` is what's hearing it, and `target` what it happened to. */
interface PageEvent<T = PageElement> {
    readonly type: string;
    readonly target: PageElement;
    readonly currentTarget: T;
    /** 1 on the way down, with `Capture` handlers and capturing listeners, 2 at what it happened to, and 3 on the way back up. */
    readonly eventPhase: number;
    readonly bubbles: boolean;
    readonly cancelable: boolean;
    readonly defaultPrevented: boolean;
    readonly isTrusted: boolean;
    readonly timeStamp: number;
    readonly nativeEvent: any;
    /** Keeps it from doing what it would by itself: a key from being typed or doing what a field does with it, a press from moving the keyboard, the wheel from scrolling, and a paste, copy or cut from happening. */
    preventDefault(): void;
    /** Keeps it from what's around what's hearing it now, `document` and `window` included. */
    stopPropagation(): void;
    stopImmediatePropagation(): void;
    isDefaultPrevented(): boolean;
    isPropagationStopped(): boolean;
    persist(): void;
}
interface PageModifierKeys {
    readonly ctrlKey: boolean;
    readonly shiftKey: boolean;
    readonly altKey: boolean;
    /** The Windows key. */
    readonly metaKey: boolean;
    getModifierState(key: 'Control' | 'Shift' | 'Alt' | 'Meta' | string): boolean;
}
/** Where the pointer was is from the window's top left in its pixels, and `offsetX` and `offsetY` from what it happened to. */
interface PageMouseEvent<T = PageElement> extends PageEvent<T>, PageModifierKeys {
    readonly clientX: number;
    readonly clientY: number;
    readonly pageX: number;
    readonly pageY: number;
    readonly x: number;
    readonly y: number;
    readonly screenX: number;
    readonly screenY: number;
    readonly offsetX: number;
    readonly offsetY: number;
    /** How far it's moved since the last time a move was said. */
    readonly movementX: number;
    readonly movementY: number;
    /** Which button: 0 the main one, 1 the middle, 2 the other, 3 back and 4 forward. */
    readonly button: number;
    /** Which are down: 1 the main one, 2 the other, 4 the middle, 8 back and 16 forward. */
    readonly buttons: number;
    /** How many clicks in a row. */
    readonly detail: number;
    readonly relatedTarget: PageElement | null;
}
interface PagePointerEvent<T = PageElement> extends PageMouseEvent<T> {
    readonly pointerId: number;
    readonly pointerType: 'mouse';
    readonly isPrimary: boolean;
    readonly width: number;
    readonly height: number;
    readonly pressure: number;
}
/** A notch of the wheel is 100 pixels, as it is on Windows. */
interface PageWheelEvent<T = PageElement> extends PageMouseEvent<T> {
    readonly deltaX: number;
    readonly deltaY: number;
    readonly deltaZ: number;
    readonly deltaMode: number;
}
interface PageKeyboardEvent<T = PageElement> extends PageEvent<T>, PageModifierKeys {
    /** What it means as the keyboard's set out: `'a'`, `'A'` with shift, `'Enter'`, `'ArrowLeft'`, `'F5'`, `'Shift'`, or `'Process'` while something's being composed. */
    readonly key: string;
    /** Where it is, whatever it means there, as `'KeyA'`, `'Digit1'` or `'ShiftLeft'`, so a shortcut by this stays in the same place on any keyboard. */
    readonly code: string;
    /** 1 for the one on the left of two, 2 for the right, 3 for the number pad. */
    readonly location: number;
    readonly repeat: boolean;
    /** Whether something's being put together by a way of typing, as Chinese, Japanese and Korean are, which an Enter then is for. */
    readonly isComposing: boolean;
    readonly keyCode: number;
    readonly which: number;
    readonly charCode: number;
}
interface PageFocusEvent<T = PageElement> extends PageEvent<T> {
    readonly relatedTarget: PageElement | null;
}
interface PageClipboardEvent<T = PageElement> extends PageEvent<T> {
    readonly clipboardData: {
        readonly types: string[];
        /** What's being pasted, as text. */
        getData(type: string): string;
        /** What a copy or cut puts on the clipboard, with `preventDefault()` so that it isn't what's selected. */
        setData(type: string, value: string): void;
        clearData(): void;
        readonly files: unknown[];
        readonly items: unknown[];
    };
}
interface PageCompositionEvent<T = PageElement> extends PageEvent<T> {
    /** What it's come to so far, or what it came to, at the end. */
    readonly data: string;
}
interface PageTransitionEvent<T = PageElement> extends PageEvent<T> {
    readonly propertyName: string;
    readonly elapsedTime: number;
    readonly pseudoElement: string;
}
interface PageAnimationEvent<T = PageElement> extends PageEvent<T> {
    readonly animationName: string;
    readonly elapsedTime: number;
    readonly pseudoElement: string;
}
interface PageChangeEvent<T = HTMLInputElement> extends PageEvent<T> {
    readonly target: PageElement & { value: string; checked: boolean; name?: string; type?: string };
}
/** A file dragged in from another program. */
interface PageFile {
    readonly name: string;
    /** Where it is, which a page doesn't say, and a program can use. */
    readonly path: string;
    readonly type: string;
    readonly size: number;
}
/**
 * Something being dragged: what `dragstart` set, for something dragged inside the app, or the files, text and links from
 * another program. `preventDefault()` in `onDragOver` or `onDragEnter` says it can be dropped there, and then `onDrop` hears it.
 */
interface PageDragEvent<T = PageElement> extends PageMouseEvent<T> {
    readonly dataTransfer: {
        dropEffect: string;
        effectAllowed: string;
        readonly types: string[];
        readonly files: PageFile[];
        readonly items: unknown[];
        /** `'text'` is `'text/plain'`, and `'url'` is `'text/uri-list'`. It can be read all through a drag, not only when it's dropped. */
        getData(format: string): string;
        setData(format: string, value: string): void;
        clearData(format?: string): void;
        setDragImage(image: unknown, x: number, y: number): void;
    };
}
interface DragEvent extends PageDragEvent {}
interface PageImageEvent<T = HTMLImageElement> extends PageEvent<T> {
    readonly target: HTMLImageElement;
}
// What `document.addEventListener` and the like are given.
interface MouseEvent extends PageMouseEvent {}
interface PointerEvent extends PagePointerEvent {}
interface WheelEvent extends PageWheelEvent {}
interface KeyboardEvent extends PageKeyboardEvent {}
interface FocusEvent extends PageFocusEvent {}
interface ClipboardEvent extends PageClipboardEvent {}
interface CompositionEvent extends PageCompositionEvent {}

/** What `document` and `window` hear, by what they're called. */
interface PageEventMap {
    click: MouseEvent;
    dblclick: MouseEvent;
    auxclick: MouseEvent;
    contextmenu: MouseEvent;
    mousedown: MouseEvent;
    mouseup: MouseEvent;
    mousemove: MouseEvent;
    pointerdown: PointerEvent;
    pointerup: PointerEvent;
    pointermove: PointerEvent;
    wheel: WheelEvent;
    keydown: KeyboardEvent;
    keyup: KeyboardEvent;
    paste: ClipboardEvent;
    copy: ClipboardEvent;
    cut: ClipboardEvent;
    compositionstart: CompositionEvent;
    compositionupdate: CompositionEvent;
    compositionend: CompositionEvent;
    dragstart: DragEvent;
    drag: DragEvent;
    dragenter: DragEvent;
    dragover: DragEvent;
    dragleave: DragEvent;
    drop: DragEvent;
    dragend: DragEvent;
}
type PageListenOptions = boolean | { capture?: boolean; once?: boolean; passive?: boolean; signal?: AbortSignal };
/**
 * Listening for what happens anywhere in the app, as a page's `document` and `window` do: on the way down to what it
 * happened to with `capture`, and on the way back up otherwise, after everything it's in. A click on what doesn't take
 * clicks is still heard, which is what "click outside to close" needs.
 */
interface PageListening {
    addEventListener<K extends keyof PageEventMap>(type: K, listener: (event: PageEventMap[K]) => void, options?: PageListenOptions): void;
    addEventListener(type: string, listener: (event: any) => void, options?: PageListenOptions): void;
    removeEventListener<K extends keyof PageEventMap>(type: K, listener: (event: PageEventMap[K]) => void, options?: PageListenOptions): void;
    removeEventListener(type: string, listener: (event: any) => void, options?: PageListenOptions): void;
}
/** There once `rsui/web` is loaded, which a page's elements do for themselves. */
declare const document: PageListening & {
    readonly body: PageElement & PageListening;
    readonly documentElement: PageElement & PageListening;
    readonly activeElement: PageElement | null;
    title: string;
};
declare const window: PageListening & typeof globalThis & {
    readonly innerWidth: number;
    readonly innerHeight: number;
    /** Whether a CSS media query holds, as `@media` has it, and `change` when that changes, as when the window's narrower than a width. */
    matchMedia(query: string): PageMediaQueryList;
};
interface PageMediaQueryList {
    readonly media: string;
    readonly matches: boolean;
    onchange: ((event: { matches: boolean; media: string }) => void) | null;
    addEventListener(type: 'change', listener: (event: { matches: boolean; media: string }) => void): void;
    removeEventListener(type: 'change', listener: (event: { matches: boolean; media: string }) => void): void;
}
interface PageResizeObserverEntry {
    readonly target: PageElement;
    readonly contentRect: { x: number; y: number; width: number; height: number; top: number; left: number; right: number; bottom: number };
    readonly borderBoxSize: readonly { inlineSize: number; blockSize: number }[];
    readonly contentBoxSize: readonly { inlineSize: number; blockSize: number }[];
}
/**
 * Told how big what it watches is each time that's different once it's been laid out, as a page's is, starting with the first
 * time. What it changes is drawn in the next frame.
 */
declare class ResizeObserver {
    constructor(callback: (entries: PageResizeObserverEntry[], observer: ResizeObserver) => void);
    observe(target: PageElement, options?: { box?: 'content-box' | 'border-box' }): void;
    unobserve(target: PageElement): void;
    disconnect(): void;
}

/** A `style` written for a page, by React's names for CSS's properties. */
type PageStyle = { [property: string]: string | number | undefined };

/** What any of a page's elements can be given. */
interface PageProps<T = PageElement> {
    key?: string | number;
    ref?: { current: T | null } | ((element: T | null) => void) | null;
    children?: import('rsui').Child;
    className?: string;
    id?: string;
    style?: PageStyle;
    title?: string;
    hidden?: boolean;
    /** 0 or more lets tab reach it, the lowest above 0 first and then 0s in the order they come; -1 only lets it be clicked to, or asked to with `focus()`. */
    tabIndex?: number;
    role?: string;
    [attribute: `data-${string}`]: string | number | boolean | undefined;
    [attribute: `aria-${string}`]: string | number | boolean | undefined;
    onClick?: (event: PageMouseEvent<T>) => void;
    onClickCapture?: (event: PageMouseEvent<T>) => void;
    onDoubleClick?: (event: PageMouseEvent<T>) => void;
    /** The other button was clicked here. `preventDefault()` does nothing, as nothing else would come up. */
    onContextMenu?: (event: PageMouseEvent<T>) => void;
    /** A click with the middle button, or back or forward. */
    onAuxClick?: (event: PageMouseEvent<T>) => void;
    onMouseDown?: (event: PageMouseEvent<T>) => void;
    onMouseDownCapture?: (event: PageMouseEvent<T>) => void;
    onMouseUp?: (event: PageMouseEvent<T>) => void;
    onMouseUpCapture?: (event: PageMouseEvent<T>) => void;
    /** At most once a frame, and while the main button's down on this, wherever the pointer goes. */
    onMouseMove?: (event: PageMouseEvent<T>) => void;
    onMouseMoveCapture?: (event: PageMouseEvent<T>) => void;
    onMouseEnter?: (event: PageMouseEvent<T>) => void;
    onMouseLeave?: (event: PageMouseEvent<T>) => void;
    onMouseOver?: (event: PageMouseEvent<T>) => void;
    onMouseOut?: (event: PageMouseEvent<T>) => void;
    onPointerDown?: (event: PagePointerEvent<T>) => void;
    onPointerDownCapture?: (event: PagePointerEvent<T>) => void;
    onPointerUp?: (event: PagePointerEvent<T>) => void;
    onPointerUpCapture?: (event: PagePointerEvent<T>) => void;
    /** At most once a frame, and while the main button's down on this, wherever the pointer goes, as a slider's thumb wants. */
    onPointerMove?: (event: PagePointerEvent<T>) => void;
    onPointerMoveCapture?: (event: PagePointerEvent<T>) => void;
    onPointerEnter?: (event: PagePointerEvent<T>) => void;
    onPointerLeave?: (event: PagePointerEvent<T>) => void;
    onPointerOver?: (event: PagePointerEvent<T>) => void;
    onPointerOut?: (event: PagePointerEvent<T>) => void;
    /** `preventDefault()` keeps what's under the pointer from scrolling. */
    onWheel?: (event: PageWheelEvent<T>) => void;
    onWheelCapture?: (event: PageWheelEvent<T>) => void;
    /** Before what has the keyboard does anything with it, so `preventDefault()` keeps a field from typing a letter or selecting everything with ctrl+a. */
    onKeyDown?: (event: PageKeyboardEvent<T>) => void;
    onKeyDownCapture?: (event: PageKeyboardEvent<T>) => void;
    onKeyUp?: (event: PageKeyboardEvent<T>) => void;
    onKeyUpCapture?: (event: PageKeyboardEvent<T>) => void;
    /** What's left of a key once what has the keyboard has done with it. */
    onKeyPress?: (event: PageKeyboardEvent<T>) => void;
    onFocus?: (event: PageFocusEvent<T>) => void;
    onBlur?: (event: PageFocusEvent<T>) => void;
    onPaste?: (event: PageClipboardEvent<T>) => void;
    onPasteCapture?: (event: PageClipboardEvent<T>) => void;
    onCopy?: (event: PageClipboardEvent<T>) => void;
    onCopyCapture?: (event: PageClipboardEvent<T>) => void;
    onCut?: (event: PageClipboardEvent<T>) => void;
    onCutCapture?: (event: PageClipboardEvent<T>) => void;
    onCompositionStart?: (event: PageCompositionEvent<T>) => void;
    onCompositionUpdate?: (event: PageCompositionEvent<T>) => void;
    onCompositionEnd?: (event: PageCompositionEvent<T>) => void;
    /** It can be dragged, once it's pressed and moved a little, which isn't a click then. */
    draggable?: boolean | 'true' | 'false';
    onDragStart?: (event: PageDragEvent<T>) => void;
    /** While it's being dragged, as the pointer moves. */
    onDrag?: (event: PageDragEvent<T>) => void;
    onDragEnd?: (event: PageDragEvent<T>) => void;
    /** `preventDefault()` here or in `onDragOver` says what's dragged can be dropped here, from inside the app or another program. */
    onDragEnter?: (event: PageDragEvent<T>) => void;
    onDragOver?: (event: PageDragEvent<T>) => void;
    onDragLeave?: (event: PageDragEvent<T>) => void;
    onDrop?: (event: PageDragEvent<T>) => void;
    onDragOverCapture?: (event: PageDragEvent<T>) => void;
    onDropCapture?: (event: PageDragEvent<T>) => void;
    /** With nothing moving, as for an agent or a test, it's as soon as it starts. */
    onTransitionEnd?: (event: PageTransitionEvent<T>) => void;
    onAnimationEnd?: (event: PageAnimationEvent<T>) => void;
}
/** A field of a page. */
interface PageFieldProps<T = HTMLInputElement> extends PageProps<T> {
    value?: string | number;
    defaultValue?: string | number;
    checked?: boolean;
    defaultChecked?: boolean;
    placeholder?: string;
    disabled?: boolean;
    readOnly?: boolean;
    name?: string;
    type?: string;
    min?: number | string;
    max?: number | string;
    step?: number | string;
    rows?: number;
    autoFocus?: boolean;
    onChange?: (event: PageChangeEvent<T>) => void;
    /** As `onChange` is. */
    onInput?: (event: PageChangeEvent<T>) => void;
}

declare const __jsx: (type: unknown, props: unknown, ...children: unknown[]) => import('rsui').Element;
declare const __Fragment: import('rsui').Component<{ children?: import('rsui').Child }>;

/** What a page's element has that rsui's own of the same name doesn't, for `span`, `button` and `input` written for a page. */
type PageExtras<T> = Omit<PageProps<T>, keyof import('rsui').ElementProps>;
/** rsui's own props for a tag a page has too, as `<button>`: its `style` is rsui's, or the page's CSS in a page. */
type SharedProps = Omit<import('rsui').ElementProps, 'style'> & { style?: import('rsui').Style | PageStyle };

declare namespace JSX {
    type Props = import('rsui').ElementProps;
    type Element = import('rsui').Element;
    /** A component can give back anything that can be a child, not only an element. */
    type ElementType = keyof IntrinsicElements | ((props: any) => import('rsui').Child) | (new (props: any) => { render(): import('rsui').Child });
    interface IntrinsicAttributes {
        key?: string | number;
    }
    /**
     * A long list, which only builds the rows in view. It's as big as layout makes it, as anything else is: `grow`, a size that's
     * a share, `position="absolute"` with all four sides, or a page's CSS around it, and how many rows fit is measured from that.
     */
    interface ListProps extends Props {
        /** How many rows there are, or in a grid, how many items. */
        count: number;
        /** How tall each one is. In a grid, how far one row's top is from the next, which takes in the `gap` between them. */
        rowHeight: number;
        /**
         * Makes it a grid that wraps by itself: its rows hold as many items this wide as fit across, with its `gap` between them,
         * and `renderRow` is given each item's index rather than a row's. Each item is `rowHeight` less the `gap` tall.
         */
        itemWidth?: number;
        /** How many to expect to fit, until it's been measured, which is as soon as it's laid out. */
        visible?: number;
        /** How many to build beyond what shows. */
        overscan?: number;
        /** A row to bring into view when this changes, as when the keyboard moves through the list. In a grid, an item. */
        reveal?: number;
        renderRow: (index: number) => import('rsui').Child;
        /**
         * Told how many rows fit, which can be part of one, and how many items fit across (1 if it isn't a grid), when it's first
         * laid out and each time that changes, as for the keyboard to go a row or a page at a time. It's told as it's laid out,
         * so what it changes is in the same frame.
         */
        onResize?: (rows: number, columns: number) => void;
    }
    interface IntrinsicElements {
        window: Props;
        group: Props;
        row: Props;
        column: Props;
        text: Props;
        /// Part of the text it's in, which can look different and wraps along with the rest.
        span: SharedProps & PageExtras<HTMLSpanElement>;
        button: SharedProps & PageExtras<HTMLButtonElement> & { type?: string };
        link: Props;
        input: SharedProps & { value?: string; placeholder?: string; placeholderColor?: string; rows?: number } & PageExtras<HTMLInputElement> & { type?: string; checked?: boolean; defaultValue?: string; defaultChecked?: boolean; onInput?: (event: PageChangeEvent<HTMLInputElement>) => void };
        checkbox: Props & { checked?: boolean };
        item: Props;
        image: Props & { src: string; fit?: 'contain' | 'cover' | 'fill' };
        dialog: Props;
        menu: Props;
        menuitem: Props;
        /** Whatever doesn't fit in it can be reached with the wheel, the keyboard or its scrollbar. */
        scroll: Props & { axis?: 'vertical' | 'horizontal' | 'both' };
        /** A long list, which only builds the rows in view. */
        list: ListProps;
        /**
         * A web page from elsewhere, in the system's browser view (WebView2), placed and sized like a `<group>`, or by the page's
         * CSS like a `<div>` in a page. See `rsui/webview` for what it says and its handle.
         */
        webview: Omit<Props, 'ref' | 'children' | 'style' | 'className'> & import('rsui/webview').WebViewProps;
        // A page's own, which are drawn as its CSS says. `span`, `button` and `input` above are a page's too, when they're
        // in a file written for a page, or have a `className` or a `type`.
        div: PageProps<HTMLDivElement>;
        p: PageProps<HTMLParagraphElement>;
        section: PageProps<HTMLElement>;
        article: PageProps<HTMLElement>;
        aside: PageProps<HTMLElement>;
        header: PageProps<HTMLElement>;
        footer: PageProps<HTMLElement>;
        main: PageProps<HTMLElement>;
        nav: PageProps<HTMLElement>;
        figure: PageProps<HTMLElement>;
        figcaption: PageProps<HTMLElement>;
        h1: PageProps<HTMLElement>;
        h2: PageProps<HTMLElement>;
        h3: PageProps<HTMLElement>;
        h4: PageProps<HTMLElement>;
        h5: PageProps<HTMLElement>;
        h6: PageProps<HTMLElement>;
        ul: PageProps<HTMLUListElement>;
        ol: PageProps<HTMLUListElement>;
        li: PageProps<HTMLLIElement>;
        dl: PageProps<HTMLElement>;
        dt: PageProps<HTMLElement>;
        dd: PageProps<HTMLElement>;
        a: PageProps<HTMLAnchorElement> & { href?: string; target?: string; rel?: string };
        strong: PageProps<HTMLElement>;
        em: PageProps<HTMLElement>;
        b: PageProps<HTMLElement>;
        i: PageProps<HTMLElement>;
        u: PageProps<HTMLElement>;
        s: PageProps<HTMLElement>;
        small: PageProps<HTMLElement>;
        mark: PageProps<HTMLElement>;
        code: PageProps<HTMLElement>;
        kbd: PageProps<HTMLElement>;
        pre: PageProps<HTMLElement>;
        blockquote: PageProps<HTMLElement>;
        sub: PageProps<HTMLElement>;
        sup: PageProps<HTMLElement>;
        hr: PageProps<HTMLElement>;
        label: PageProps<HTMLLabelElement> & { htmlFor?: string };
        form: PageProps<HTMLFormElement> & { onSubmit?: (event: PageEvent<HTMLFormElement>) => void };
        fieldset: PageProps<HTMLElement>;
        legend: PageProps<HTMLElement>;
        textarea: PageFieldProps<HTMLTextAreaElement>;
        select: PageFieldProps<HTMLSelectElement>;
        option: PageProps<HTMLElement> & { value?: string | number };
        img: PageProps<HTMLImageElement> & {
            src?: string;
            alt?: string;
            width?: number | string;
            height?: number | string;
            /** Once its size is known, as soon as what's running now has finished. */
            onLoad?: (event: PageImageEvent) => void;
            /** It can't be read. */
            onError?: (event: PageImageEvent) => void;
        };
        table: PageProps<HTMLElement>;
        thead: PageProps<HTMLElement>;
        tbody: PageProps<HTMLElement>;
        tfoot: PageProps<HTMLElement>;
        tr: PageProps<HTMLElement>;
        th: PageProps<HTMLElement>;
        td: PageProps<HTMLElement>;
        details: PageProps<HTMLElement> & { open?: boolean };
        summary: PageProps<HTMLElement>;
        // A picture, written as SVG is, in a page or in rsui's own window alike: drawn as `Svg` and its shapes are, which
        // these are. What says how something else is drawn (`<defs>`, gradients, `<clipPath>`, masks, filters) isn't drawn.
        svg: SvgProps & {
            viewBox?: string;
            width?: number | string;
            height?: number | string;
            xmlns?: string;
            /** `none` stretches it to fill its box, rather than keeping its shape. */
            preserveAspectRatio?: string;
            /** What `currentColor` is in it. */
            color?: string;
        };
        g: SvgProps;
        path: SvgProps & { d: string };
        circle: SvgProps & { cx?: number | string; cy?: number | string; r: number | string };
        ellipse: SvgProps & { cx?: number | string; cy?: number | string; rx: number | string; ry: number | string };
        rect: SvgProps & { x?: number | string; y?: number | string; width: number | string; height: number | string; rx?: number | string; ry?: number | string };
        line: SvgProps & { x1?: number | string; y1?: number | string; x2?: number | string; y2?: number | string };
        polyline: SvgProps & { points: string };
        polygon: SvgProps & { points: string };
    }
    /** How a page's `<svg>` or a shape in it is drawn, as SVG's attributes say, which CSS goes over. On the `<svg>` or a `<g>`, it's
     * for all that's in it. */
    interface SvgProps extends PageProps<PageElement> {
        /** `currentColor` is the `<svg>`'s `color`, and `none` is nothing. */
        fill?: string;
        stroke?: string;
        strokeWidth?: number | string;
        strokeLinecap?: 'butt' | 'round' | 'square';
        /** How a line's corners are drawn. `miter` if nothing says. */
        strokeLinejoin?: 'miter' | 'round' | 'bevel';
        /** How far a mitred corner may reach, in widths of the line, before it's bevelled instead: 4 if nothing says. */
        strokeMiterlimit?: number | string;
        strokeDasharray?: string | number | number[];
        strokeDashoffset?: number | string;
        /** As SVG writes it: `'translate(4 2) rotate(45)'`. */
        transform?: string;
    }
    interface ElementChildrenAttribute {
        children: {};
    }
}

declare module 'os' {
    export const EOL: string;
    export function platform(): string;
    export function homedir(): string;
    export function tmpdir(): string;
    export function hostname(): string;
    export function totalmem(): number;
    export function freemem(): number;
    export function uptime(): number;
    export function availableParallelism(): number;
    export function cpus(): unknown[];
    export function release(): string;
    export function userInfo(): { username: string; homedir: string };
}

declare module 'crypto' {
    interface Hash {
        update(data: string | Uint8Array): this;
        digest(): Buffer;
        digest(encoding: BufferEncoding): string;
    }
    export function createHash(name: 'md5' | 'sha1' | 'sha256' | 'sha384' | 'sha512' | string): Hash;
    export function createHmac(name: string, key: string | Uint8Array): Hash;
    export function randomBytes(size: number): Buffer;
    export function randomUUID(): string;
    export function randomInt(below: number): number;
    export function randomInt(from: number, below: number): number;
    export function timingSafeEqual(a: Uint8Array, b: Uint8Array): boolean;
}

declare module 'stream' {
    class Base {
        on(name: string, listener: (...args: any[]) => void): this;
        once(name: string, listener: (...args: any[]) => void): this;
        off(name: string, listener: (...args: any[]) => void): this;
        emit(name: string, ...args: unknown[]): boolean;
        destroy(error?: Error): this;
        pipe<T extends Writable>(target: T, options?: { end?: boolean }): T;
        readonly destroyed: boolean;
    }
    interface ReadableOptions {
        objectMode?: boolean;
        encoding?: BufferEncoding;
        highWaterMark?: number;
        read?(this: Readable, size: number): void;
    }
    export class Readable extends Base implements AsyncIterable<any> {
        constructor(options?: ReadableOptions);
        static from(from: Iterable<unknown> | AsyncIterable<unknown> | string): Readable;
        push(chunk: unknown, encoding?: BufferEncoding): boolean;
        read(): any;
        pause(): this;
        resume(): this;
        setEncoding(encoding: BufferEncoding): this;
        [Symbol.asyncIterator](): AsyncIterableIterator<any>;
    }
    interface WritableOptions {
        objectMode?: boolean;
        highWaterMark?: number;
        write?(this: Writable, chunk: any, encoding: string, done: (error?: Error | null) => void): void;
        final?(this: Writable, done: (error?: Error | null) => void): void;
    }
    export class Writable extends Base {
        constructor(options?: WritableOptions);
        write(chunk: unknown, encoding?: BufferEncoding, done?: (error?: Error | null) => void): boolean;
        end(chunk?: unknown, encoding?: BufferEncoding, done?: () => void): this;
    }
    export class Duplex extends Readable {
        constructor(options?: ReadableOptions & WritableOptions);
        write(chunk: unknown, encoding?: BufferEncoding, done?: (error?: Error | null) => void): boolean;
        end(chunk?: unknown, encoding?: BufferEncoding, done?: () => void): this;
    }
    export class Transform extends Duplex {
        constructor(options?: ReadableOptions & WritableOptions & {
            transform?(this: Transform, chunk: any, encoding: string, done: (error?: Error | null, value?: unknown) => void): void;
            flush?(this: Transform, done: (error?: Error | null, value?: unknown) => void): void;
        });
    }
    export class PassThrough extends Transform {}
    export function pipeline(...streams: any[]): any;
    export function finished(stream: unknown, done: (error?: Error) => void): () => void;
}

declare module 'child_process' {
    interface RunOptions {
        cwd?: string;
        env?: Record<string, string | undefined>;
        encoding?: BufferEncoding | 'buffer' | null;
        shell?: boolean;
        timeout?: number;
        input?: string | Uint8Array;
    }
    export class ChildProcess {
        readonly pid: number | undefined;
        readonly stdin: import('stream').Writable;
        readonly stdout: import('stream').Readable;
        readonly stderr: import('stream').Readable;
        readonly exitCode: number | null;
        readonly killed: boolean;
        kill(): boolean;
        on(name: 'close' | 'exit', listener: (code: number | null, signal: string | null) => void): this;
        on(name: 'error', listener: (error: Error) => void): this;
        on(name: string, listener: (...args: any[]) => void): this;
    }
    export function spawn(file: string, args?: string[], options?: RunOptions): ChildProcess;
    export function exec(command: string, done: (error: Error | null, stdout: string, stderr: string) => void): ChildProcess;
    export function exec(command: string, options: RunOptions, done: (error: Error | null, stdout: string, stderr: string) => void): ChildProcess;
    export function execFile(file: string, args: string[], done: (error: Error | null, stdout: string, stderr: string) => void): ChildProcess;
    export function execFile(file: string, args: string[], options: RunOptions, done: (error: Error | null, stdout: string, stderr: string) => void): ChildProcess;
    export function execSync(command: string, options?: RunOptions): any;
    export function execFileSync(file: string, args?: string[], options?: RunOptions): any;
    export function spawnSync(file: string, args?: string[], options?: RunOptions): { status: number | null; stdout: any; stderr: any; error?: Error };
}
