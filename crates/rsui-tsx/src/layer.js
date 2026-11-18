// What `import ... from 'rsui'` gives: elements, hooks and ready-made widgets. Rust does everything else.
//
// `__jsx` only describes what's wanted. Rust walks the description after a component returns, makes the real elements,
// and mounts each component in it as a component of rsui's own runtime. So a component's hooks live here, while when it
// renders, and what changed as a result, are the runtime's business.

const host = globalThis.__rsuiHost;
delete globalThis.__rsuiHost;

const NO_PROPS = Object.freeze({});

function Fragment(props) {
    return props.children;
}

globalThis.__Fragment = Fragment;

// What a component gave back, as a list, to be drawn in another order (`order`, of where each is in `kids`), each where React has
// it, so that nothing starts again: what code moved of what React drew, as cmdk sorts its items (see `moveDrawn` in web.js).
// Rust walks it as it would the list itself (`child_in` in bridge.rs).
globalThis.__Reordered = Object.freeze({ reordered: true });

// What a component's element says when the app's own JSX didn't make it, as when a library did (see `__jsxMemo`): it's only the
// same as itself when it's in what another component's given, as what it holds may have been changed in place, as TanStack
// Table's cells have it (see `Vm::props_equivalent`).
const NOT_KEPT = Symbol('rsui.notKept');

globalThis.__jsx = function (type, props, kids, mayHaveKey) {
    const key = mayHaveKey === true ? props.key : undefined;
    if (typeof type === 'function' && type !== Fragment) {
        if (kids !== undefined && kids !== null && kids.length > 0) {
            props = { ...props, children: kids.length === 1 ? kids[0] : kids };
        } else if (props === null) {
            props = NO_PROPS;
        }
        return { type, key, props, kids: null, memo: NOT_KEPT };
    }
    // A context is its own provider, as `<Theme value="dark">`, which is how React 19 has it.
    if (typeof type === 'object' && type !== null && typeof type.Provider === 'function') {
        return __jsx(type.Provider, props, kids, mayHaveKey);
    }
    return { type, key, props, kids };
};

// What JSX in an app's own files calls, where nothing says 'use no memo': `__jsx`, with a component's element marked `memo`, so
// that it doesn't render again when what it's given does the same as before, as React Compiler has it by keeping the element
// (see `JsProps` in bridge.rs). Elements made anywhere else, as in `node_modules`, are as `__jsx` has them.
// It's called by a name of its own here, as the compiler takes `__jsxMemo` in a file that isn't the app's, as this one, to be `__jsx`.
function jsxMemo(type, props, kids, mayHaveKey) {
    // A tag says so too, for what a page makes of it (see `webDescription`).
    if (typeof type === 'string') {
        return { type, key: mayHaveKey === true ? props.key : undefined, props, kids, memo: true };
    }
    if (typeof type !== 'function' || type === Fragment) {
        return __jsx(type, props, kids, mayHaveKey);
    }
    const key = mayHaveKey === true ? props.key : undefined;
    if (kids !== undefined && kids !== null && kids.length > 0) {
        props = { ...props, children: kids.length === 1 ? kids[0] : kids };
    } else if (props === null) {
        props = NO_PROPS;
    }
    // A page's element says which version of the page's CSS it was made under, as it's only compared with one made under the
    // same (see `JsProps` in bridge.rs).
    return { type, key, props, kids: null, memo: type.__rsuiEquivalent === true ? pageMark() : true };
}

// The page's own mark for an element of it the app's JSX made: see `__pageMark` in web.js.
let pageMarking = null;
function pageMark() {
    pageMarking ??= require('rsui/web').__pageMark;
    return pageMarking();
}

globalThis.__jsxMemo = jsxMemo;

// What JSX calls in a file written for a page, which has a tag rsui has too, as a `<button>` beside a `<div>`: each of a page's tags
// is the page's, drawn as its CSS says, and anything else is as `__jsx` has it. `rsui/web` is loaded the first time.
globalThis.__jsxPage = function (type, props, kids, mayHaveKey) {
    if (typeof type === 'string') {
        pageElement ??= require('rsui/web').__pageElement;
        type = pageElement(type) ?? type;
    }
    return __jsx(type, props, kids, mayHaveKey);
};

globalThis.__jsxPageMemo = function (type, props, kids, mayHaveKey) {
    if (typeof type === 'string') {
        pageElement ??= require('rsui/web').__pageElement;
        type = pageElement(type) ?? type;
    }
    return jsxMemo(type, props, kids, mayHaveKey);
};

// What `import './app.css'` does: the page's CSS, as a bundler has it, which is `rsui/web`'s. `import styles from './a.module.css'`
// gives each class by its own name, as there's nothing for them to clash with here.
globalThis.__rsuiCss = (text, file) => require('rsui/web').__addCssFile(text, file);

// `__jsx` as the widgets below can write it, without knowing as much as what compiles JSX does.
function element(type, props, ...kids) {
    return __jsx(type, props, kids, props !== null && 'key' in props);
}

// The component being rendered, and where its hooks have got to.
let current = null;
// Whether code is being swapped, when a hook that doesn't line up means starting that component's state again.
let reloading = false;
// Whether something's looking at why components render. While it is, each hook keeps where it's called from, which
// names it, and setting state says where it was set. It costs one check while it isn't.
let tracing = false;

const RESTART = { restart: true };

// When an effect runs in a commit, as React has them: what CSS-in-JS inserts with, what's laid out and measured with, and the rest.
const INSERTION = 0;
const LAYOUT = 1;
const PASSIVE = 2;

// What a component with no hooks has, as most have none. Nothing adds to this.
const NO_HOOKS = Object.freeze([]);

// What an instance's `flags` say, as one number rather than a property each, as every property past an object's first four
// takes 8 bytes more of its own, for each component there is.
// Whether it's finished rendering once, after which it can't have any new hooks.
const RENDERED = 1;
const DEAD = 2;
// Whether what's here needs more than calling it, which is for 'react' to do.
const ADAPTED = 4;
// Whether this is what a `Suspense` shows while it waits.
const FALLBACK = 8;
// Why it's to render again: whether its state was set, and whether something else has said it must, which `render` needs to know
// whether what it gave back last time will do.
const STATE_SET = 16;
const FORCED = 32;
// Whether it's in the list of what has effects to run once this commit's done.
const COMMITTING = 64;
// Only compiler participants have memo-cache data, kept outside the ordinary hook list.
const COMPILER_CACHE = 128;

// Where the component rendering now has got to in its hooks.
let hookAt = 0;

function newInstance(slot, parent) {
    const activity = parent === null ? null : activityBelow(parent);
    const instance = {
        slot,
        parent,
        type: null,
        hooks: NO_HOOKS,
        flags: 0,
        // What effects are due to run, once the tree is up to date.
        due: null,
        provides: null,
        watching: null,
        // What it said it read of contexts that only tell what reads what's changed (see `readsHere`), by context.
        reads: null,
        // What it was last given and what it gave back.
        props: undefined,
        output: null,
        everything: 0,
        // The `Activity` it's in, if it's in one.
        activity,
        // What few components have, made when one does: see `moreOf`.
        more: null,
        // The page's element it draws, which 'rsui/web' says, here from the start as adding it would make room for many more.
        __pageLive: undefined,
        // The page's elements drawn in place in what it gives back, which 'rsui/web' keeps: see `placeElement` there.
        placed: undefined,
        // For what stands for one of those, the component whose output it's in: see `newFrame`.
        placedIn: undefined,
    };
    if (activity !== null) {
        activity.members.add(instance);
    }
    return instance;
}

/// What stands for a page's plain element that's drawn in place in the output of `owner`, as a host element, rather than as a
/// component of its own: see `placeElement` in 'rsui/web'. It's where the element is among the components, for what's in it and
/// what it reads of what's above it, as an instance would be, and what it passes down is on it as a component's is, but it has no
/// hooks, effects or slot of its own, and nothing renders it: what it reads changing has its owner build what it gave back again,
/// without calling it, and the element drawn again then (see `placedChanged`).
function newFrame(parent, owner, type, live) {
    const frame = newInstance(owner.slot, parent);
    frame.type = type;
    frame.placedIn = owner;
    frame.__pageLive = live;
    return frame;
}

/// Renders a page's element drawn in place as `frame` stands for it, which is what's rendering while it does.
function renderPlaced(frame, type, props) {
    const before = current;
    const beforeAt = hookAt;
    current = frame;
    hookAt = 0;
    frame.flags &= ~FORCED;
    try {
        return type(props);
    } finally {
        current = before;
        hookAt = beforeAt;
    }
}

/// Lets go of what stands for an element drawn in place once it's gone.
function forgetFrame(frame) {
    frame.activity?.members.delete(frame);
    frame.flags |= DEAD;
    runCleanups(frame);
}

/// What's said when what an element drawn in place reads changes: its owner builds what it gave back again, without calling it, as
/// its state was set (see `render`), and the element's drawn again as it's come to, unless that's already under way and hasn't
/// reached it.
let placedChanged = null;

/// What few components have: what takes in an error from something below, if it's an error boundary; what's being waited for
/// below, if it's a `Suspense`; what it's waiting for itself, when there's no `Suspense` around it to wait instead; and the
/// `Activity` what's in it is in, if it's one. A `Window`'s root keeps its window here too, and tracing where contexts are read.
function moreOf(instance) {
    return instance.more ??= { catches: null, suspense: null, waitingFor: undefined, below: undefined, reorder: undefined };
}

/// The `Activity` what's in `instance` is in, which is the one it's in, unless it's an `Activity` itself.
function activityBelow(instance) {
    return instance.more?.below ?? instance.activity;
}

// How to render a class or something with default props, from 'react', if that's been loaded.
let renderAdapted = null;

function boundaryAbove(instance) {
    let at = instance.parent;
    while (at !== null && (at.more === null || at.more.catches === null)) {
        at = at.parent;
    }
    return at;
}

/// Makes `instance` an error boundary, which `catches` is given what's thrown below it, as 'react' does for a class that says it is.
function catchWith(instance, catches) {
    moreOf(instance).catches = catches;
}

/// What's around `instance`, from it up, as React's `componentStack` says it, for what an error boundary is told.
function componentStack(instance) {
    let stack = '';
    for (let at = instance; at !== null; at = at.parent) {
        const type = at.type;
        const name = type === null ? '' : type.displayName || type.name || '';
        if (name !== '' && type !== Fragment) {
            stack += '\n    in ' + name;
        }
    }
    return stack;
}

/// Sends what went wrong in `instance`, outside of rendering it, to the error boundary above it, as React 19 does with what's
/// thrown by an effect, a ref or a lifecycle. With none to take it, it's given back, for what called this to report.
function caughtOutside(instance, error) {
    const boundary = boundaryAbove(instance);
    if (boundary === null || (boundary.flags & DEAD) !== 0) {
        return error;
    }
    boundary.more.catches(error, { componentStack: componentStack(instance) });
    return undefined;
}

// What says nothing's gone wrong, as anything at all can be thrown.
const NO_ERROR = Object.freeze({});

/// The first error that nothing took, of what's gone wrong so far and what's just gone wrong, if anything has.
const firstUncaught = (so, left) => (so === NO_ERROR && left !== undefined ? left : so);

function runCleanups(instance) {
    let uncaught = NO_ERROR;
    // Layout effects are undone before passive ones, as React does when something goes.
    for (let phase = INSERTION; phase <= PASSIVE; phase++) {
        for (const hook of instance.hooks) {
            if (hook.cleanup && hook.kind === phase) {
                const cleanup = hook.cleanup;
                hook.cleanup = null;
                try {
                    cleanup();
                } catch (error) {
                    uncaught = firstUncaught(uncaught, caughtOutside(instance, error));
                }
            }
        }
    }
    if (instance.watching) {
        for (const provider of instance.watching) {
            unwatch(provider, instance);
        }
        instance.watching = null;
    }
    if (uncaught !== NO_ERROR) {
        throw uncaught;
    }
}

/// What a component gave back, in the order code has moved it to, if it has: see `__reorderOutput`.
function reordered(instance, output) {
    const more = instance.more;
    if (more === null || more.reorder === undefined) {
        return output;
    }
    const now = more.reorder(output);
    if (now === output) {
        more.reorder = undefined;
    }
    return now;
}

/// Whether every state a component has is what it was when it last rendered.
function statesAsShown(instance) {
    for (const hook of instance.hooks) {
        if ((hook.kind === 'state' || hook.kind === 'reducer') && !Object.is(hook.value, hook.shown)) {
            return false;
        }
    }
    return true;
}

// Goes up whenever everything has to render again, as when the system goes from light to dark, whatever it was given.
let everything = 0;

function render(instance, type, props) {
    // Only its state was set, and that's all as it was when it last rendered, as when a callback ref is given null and then its
    // element again: what it gave back then is what it would give now, so it isn't called, as React bails out. Otherwise a ref
    // that sets state would render it for ever.
    if ((instance.flags & (STATE_SET | FORCED)) === STATE_SET && instance.type === type && props === instance.props && instance.everything === everything
        && !reloading && statesAsShown(instance)) {
        instance.flags &= ~STATE_SET;
        return reordered(instance, instance.output);
    }
    instance.flags &= ~(STATE_SET | FORCED);
    instance.everything = everything;
    if ((instance.flags & COMPILER_CACHE) !== 0 && (reloading || instance.type !== type)) {
        clearCompilerCache(instance);
    }
    if (instance.type !== type) {
        if (instance.type !== null) {
            globalThis.__rsuiFlowInstance?.(instance.type, instance, false);
        }
        // Another kind of component in the same place starts again, unless it's the same one in a new version of the file.
        if (instance.type !== null && !reloading) {
            runCleanups(instance);
            instance.hooks = NO_HOOKS;
            instance.due = null;
            instance.flags &= ~RENDERED;
            // What it provided was the other kind's, as one context's Provider in the place of another's is a new provider.
            for (let provider = instance.provides ?? null; provider !== null; provider = provider.next) {
                for (const watcher of watchersOf(provider)) {
                    markDirty(watcher);
                }
            }
            instance.provides = null;
        }
        instance.type = type;
        globalThis.__rsuiFlowInstance?.(type, instance, true);
        // Finding this out only happens when the kind changes, and there's nothing to find out without 'react'.
        if (renderAdapted !== null && (type.prototype?.isReactComponent !== undefined || type.defaultProps !== undefined)) {
            instance.flags |= ADAPTED;
        } else {
            instance.flags &= ~ADAPTED;
        }
    }
    const before = current;
    const beforeAt = hookAt;
    current = instance;
    try {
        for (;;) {
            hookAt = 0;
            if ((instance.flags & COMPILER_CACHE) !== 0) {
                instance.more.compilerCache.index = 0;
            }
            try {
                const output = (instance.flags & ADAPTED) !== 0 ? renderAdapted(type, props, instance) : type(props);
                if (hookAt !== instance.hooks.length) {
                    if (!reloading) {
                        throw new Error('A component has to call the same hooks on every render: '
                            + (type.name || 'this one') + ' called ' + hookAt + ' and not '
                            + instance.hooks.length);
                    }
                    throw RESTART;
                }
                if ((instance.flags & RENDERED) === 0) {
                    instance.flags |= RENDERED;
                    // Its hooks are all there now, and no more can be added, so they're kept in no more room than they take.
                    if (instance.hooks.length > 4) {
                        instance.hooks = instance.hooks.slice();
                    }
                }
                instance.props = props;
                instance.output = output;
                // Whether a kind of component gives back nothing, as CSS-in-JS's own `Insertion` does, or something, which
                // 'rsui/web' goes by when it finds out what an element a component makes is: see `learnFlow` there.
                const said = output === null || output === undefined || output === false ? '__gaveNothing' : '__gaveSomething';
                if (type[said] !== true) {
                    try {
                        type[said] = true;
                    } catch {}
                }
                // What gives back only text, as Ant Design's Typography does inside its heading, goes along a line, which
                // 'rsui/web' is told, for what it's in to be drawn again with that: see `learnFlow` there. A context's provider
                // only hands down what it's given, which is text in one place and blocks in another, so nothing's learned of it:
                // 'rsui/web' wraps every component in its own on a page that says where things are among the rest, and one that
                // was once around text put shadcn's Breadcrumb and Menubar on a line.
                if (type.context === undefined && (type.__webFlowSeen === undefined || type.__webText === true) && onlyText(output, 0)) {
                    globalThis.__rsuiLearnedText?.(type, instance);
                }
                return reordered(instance, output);
            } catch (error) {
                // An abandoned attempt must never leave partial memo writes for its retry.
                clearCompilerCache(instance);
                instance.props = undefined;
                instance.output = null;
                if (isThenable(error)) {
                    // It's waiting for something, which `use` says by throwing it, as code written for React's Suspense does too.
                    instance.due = null;
                    suspend(instance, error);
                    return null;
                }
                if (error !== RESTART) {
                    const boundary = boundaryAbove(instance);
                    if (boundary === null) {
                        throw error;
                    }
                    // What's above it says what to show instead, when it next renders.
                    instance.due = null;
                    boundary.more.catches(error, { componentStack: componentStack(instance) });
                    return null;
                }
                runCleanups(instance);
                instance.hooks = NO_HOOKS;
                instance.due = null;
                instance.flags &= ~RENDERED;
            }
        }
    } finally {
        current = before;
        hookAt = beforeAt;
    }
}

// What has effects to run once the commit that's going on is done, in the order its components finished rendering, which is what's
// inside before what's around it, as React runs them. Rust says when that is: see `commit_effects` in `bridge.rs`.
let committed = [];
// Which kinds of effect are among those, so that a commit with only `useEffect`s doesn't look for the others.
let phasesDue = 0;

/// Puts a component that's rendered with effects to run in the list for when the commit's done.
function flushEffects(instance) {
    if (instance.due !== null && (instance.flags & (COMMITTING | DEAD)) === 0) {
        instance.flags |= COMMITTING;
        committed.push(instance);
    }
}

/// Undoes what each of `instance`'s effects of this kind that's to run again did last time.
function undoEffects(instance, phase, uncaught) {
    const due = instance.due;
    if (due === null) {
        return uncaught;
    }
    for (const hook of due) {
        if (hook.kind === phase && hook.cleanup) {
            const cleanup = hook.cleanup;
            hook.cleanup = null;
            try {
                cleanup();
            } catch (error) {
                uncaught = firstUncaught(uncaught, caughtOutside(instance, error));
            }
        }
    }
    return uncaught;
}

function runEffects(instance, phase, uncaught) {
    const due = instance.due;
    // What's in a hidden `Activity` keeps its effects for when it's shown, apart from what inserts CSS.
    if (due === null || (phase !== INSERTION && instance.activity !== null && isHidden(instance.activity))) {
        return uncaught;
    }
    for (const hook of due) {
        const run = hook.run;
        // Rendering twice before this runs puts it here twice, and only the first of those has anything to do.
        if (hook.kind !== phase || run === null) {
            continue;
        }
        // What it holds on to has no reason to stay alive until it next runs.
        hook.run = null;
        try {
            const result = run();
            hook.cleanup = typeof result === 'function' ? result : null;
        } catch (error) {
            uncaught = firstUncaught(uncaught, caughtOutside(instance, error));
        }
    }
    return uncaught;
}

/// What has effects to run, each before anything it's inside, as React's commit has them whatever rendered first. A component that
/// rendered again on its own in the same round as one it's inside comes after it in the list (Radix's FocusScope, once its ref has
/// set its state, and the menu items its RovingFocusGroup's state renders again), and its effects are to run first, so the items
/// are back in their collection when FocusScope's effect focuses the menu. Most commits are in order already, found in one pass.
function insideFirst(instances) {
    if (instances.length < 2) {
        return instances;
    }
    const places = new Map();
    for (let at = 0; at < instances.length; at++) {
        places.set(instances[at], at);
    }
    // The first place in the list of anything from `instance` up, which is the same however far below it's asked from, so it's
    // worked out once for each.
    const firsts = new Map();
    const firstFrom = (instance) => {
        let first = Infinity;
        const walked = [];
        for (let at = instance; at !== null && at !== undefined; at = at.parent) {
            const known = firsts.get(at);
            if (known !== undefined) {
                first = known;
                break;
            }
            walked.push(at);
        }
        for (let index = walked.length - 1; index >= 0; index--) {
            const one = walked[index];
            const place = places.get(one);
            if (place !== undefined && place < first) {
                first = place;
            }
            firsts.set(one, first);
        }
        return first;
    };
    let inOrder = true;
    for (let at = 0; at < instances.length && inOrder; at++) {
        const parent = instances[at].parent;
        if (parent !== null && parent !== undefined && firstFrom(parent) < at) {
            inOrder = false;
        }
    }
    if (inOrder) {
        return instances;
    }
    const ordered = [];
    for (const instance of instances) {
        const around = new Set();
        for (let at = instance.parent; at !== null && at !== undefined; at = at.parent) {
            around.add(at);
        }
        const before = ordered.findIndex((one) => around.has(one));
        if (before < 0) {
            ordered.push(instance);
        } else {
            ordered.splice(before, 0, instance);
        }
    }
    return ordered;
}

/// Runs the effects of what this commit rendered, as React does: each component's insertion effects, as CSS-in-JS adds its rules
/// with, then every layout effect that's to be undone, then every layout effect, refs included, and then the same for passive
/// effects. It's all before what changed is drawn, so state set in any of them is drawn with it. What one of them throws goes to
/// the error boundary above it, and the rest still run. What no boundary takes is thrown once they have.
function commitEffects() {
    const instances = insideFirst(committed);
    // What's drawn in place has its refs given their elements, and is on the page from now, before any layout effect runs, as
    // React has a host element's ref: see `placedCommit` in 'rsui/web'.
    const placedCommit = module.exports.__placedCommit;
    if (instances.length === 0) {
        placedCommit?.();
        return;
    }
    const phases = phasesDue;
    committed = [];
    phasesDue = 0;
    let uncaught = NO_ERROR;
    if ((phases & (1 << INSERTION)) !== 0) {
        for (const instance of instances) {
            uncaught = undoEffects(instance, INSERTION, uncaught);
            uncaught = runEffects(instance, INSERTION, uncaught);
        }
    }
    try {
        placedCommit?.();
    } catch (error) {
        uncaught = firstUncaught(uncaught, error);
    }
    // What `rsui/web` attaches once what's committed has rendered and before any layout effect runs, as React attaches refs then:
    // the refs of the parts of a page's `<svg>`, found as it renders (see `keepSvgElements`).
    const beforeLayout = module.exports.__beforeLayout;
    if (typeof beforeLayout === 'function') {
        try {
            beforeLayout();
        } catch (error) {
            uncaught = firstUncaught(uncaught, error);
        }
    }
    for (let phase = LAYOUT; phase <= PASSIVE; phase++) {
        if ((phases & (1 << phase)) === 0) {
            continue;
        }
        for (const instance of instances) {
            uncaught = undoEffects(instance, phase, uncaught);
        }
        for (const instance of instances) {
            uncaught = runEffects(instance, phase, uncaught);
        }
    }
    for (const instance of instances) {
        instance.due = null;
        instance.flags &= ~COMMITTING;
    }
    if (uncaught !== NO_ERROR) {
        throw uncaught;
    }
}

function unmount(instance) {
    if (instance.placed !== undefined) {
        module.exports.__placedOwnerGone?.(instance);
    }
    clearCompilerCache(instance);
    globalThis.__rsuiFlowInstance?.(instance.type, instance, false);
    instance.activity?.members.delete(instance);
    instance.flags |= DEAD;
    instance.due = null;
    instance.output = null;
    runCleanups(instance);
}

// A portal into something that isn't the page, as React Aria's collections are made: a document of the library's own, which it
// builds its list of items from. What's in it is rendered as components are anywhere, with their hooks and what's above them,
// but nothing's drawn: each element is a node that document makes, put in it in the order they're written, with its props and
// its ref, as react-dom does with a container of any kind. When something in it changes, the portal renders again, and goes
// through what's in it, rendering only what has to be.

const FOREIGN_SKIPPED = new Set(['children', 'key', 'ref', 'dangerouslySetInnerHTML', 'suppressContentEditableWarning', 'suppressHydrationWarning']);

/// What's in something, as one list, each with the key it's matched by from one render to the next.
function foreignItems(children, prefix, out) {
    if (children === null || children === undefined || typeof children === 'boolean' || children === '') {
        return out;
    }
    if (Array.isArray(children)) {
        children.forEach((child, at) => {
            const key = child !== null && typeof child === 'object' && child.key !== null && child.key !== undefined ? '$' + child.key : String(at);
            foreignItems(child, prefix + ':' + key, out);
        });
        return out;
    }
    if (typeof children === 'object' && children.type === Fragment) {
        return foreignItems(children.kids ?? children.props?.children, prefix + (children.key !== null && children.key !== undefined ? ':$' + children.key : ''), out);
    }
    out.push({ key: prefix, element: children });
    return out;
}

function foreignProps(node, before, after) {
    const set = (name, value) => {
        const attribute = name === 'className' ? 'class' : name === 'htmlFor' ? 'for' : name;
        try {
            if (value === null || value === undefined || value === false) {
                node.removeAttribute?.(attribute);
            } else {
                node.setAttribute?.(attribute, value === true ? '' : String(value));
            }
        } catch {}
    };
    for (const name of Object.keys(before)) {
        if (!(name in after) && !FOREIGN_SKIPPED.has(name) && !/^on[A-Z]/.test(name) && name !== 'style') {
            set(name, null);
        }
    }
    for (const name of Object.keys(after)) {
        const value = after[name];
        if (FOREIGN_SKIPPED.has(name) || /^on[A-Z]/.test(name) || Object.is(before[name], value)) {
            continue;
        }
        if (name === 'style') {
            try {
                const style = node.style;
                for (const [key, said] of Object.entries(value ?? {})) {
                    style[key] = said ?? '';
                }
            } catch {}
            continue;
        }
        set(name, value);
    }
}

/// The nodes something puts in what it's in, in order: a component's are what it rendered.
function foreignNodes(records, out = []) {
    for (const record of records) {
        if (record.node !== undefined) {
            out.push(record.node);
        } else {
            foreignNodes(record.children, out);
        }
    }
    return out;
}

/// Puts what's in a node in it in this order, moving only what isn't where it should be.
function foreignOrder(parent, nodes) {
    let next = null;
    for (let at = nodes.length - 1; at >= 0; at--) {
        const node = nodes[at];
        if (node.parentNode !== parent || node.nextSibling !== next) {
            try {
                parent.insertBefore(node, next);
            } catch {}
        }
        next = node;
    }
}

function foreignGone(record, top) {
    for (const child of record.children) {
        foreignGone(child, top && record.node === undefined);
    }
    if (record.instance !== undefined) {
        unmount(record.instance);
    }
    if (record.node !== undefined) {
        record.refUndo?.();
        record.refUndo = undefined;
        if (top) {
            try {
                record.node.parentNode?.removeChild(record.node);
            } catch {}
        }
    }
}

/// What's in `parent` (a node, or a component's instance) as these elements are now, from what it was.
function foreignList(state, owner, before, children, doc) {
    const old = new Map();
    for (const record of before) {
        old.set(record.key, record);
    }
    const out = [];
    for (const { key, element } of foreignItems(children, '', [])) {
        const kind = typeof element === 'string' || typeof element === 'number' ? '#text' : element?.type;
        let record = old.get(key);
        if (record !== undefined && record.kind !== kind) {
            foreignGone(record, true);
            record = undefined;
        }
        old.delete(key);
        if (kind === '#text') {
            const text = String(element);
            if (record === undefined) {
                if (typeof doc.createTextNode !== 'function') {
                    continue;
                }
                record = { key, kind, node: doc.createTextNode(text), text, children: [] };
            } else if (record.text !== text) {
                record.text = text;
                record.node.textContent = text;
            }
        } else if (typeof kind === 'string') {
            const props = element.props ?? NO_PROPS;
            if (record === undefined) {
                record = { key, kind, node: doc.createElement(kind), props: NO_PROPS, ref: undefined, refUndo: undefined, children: [] };
            }
            foreignProps(record.node, record.props, props);
            record.props = props;
            record.children = foreignList(state, owner, record.children, element.kids ?? props.children, doc);
            foreignOrder(record.node, foreignNodes(record.children));
            const ref = props.ref ?? element.ref;
            if (ref !== record.ref) {
                record.refUndo?.();
                record.ref = ref;
                record.refUndo = attachRef(ref, record.node);
            }
        } else if (typeof kind === 'function') {
            const props = element.props ?? NO_PROPS;
            if (record === undefined) {
                // It's drawn again with the portal, which is what Rust knows about.
                record = { key, kind, instance: newInstance(state.slot, owner), children: [] };
            }
            const instance = record.instance;
            const output = (instance.flags & (RENDERED | FORCED | STATE_SET)) !== RENDERED || instance.props !== props || instance.everything !== everything
                ? render(instance, kind, props)
                : instance.output;
            record.children = foreignList(state, instance, record.children, output, doc);
            flushEffects(instance);
        } else {
            continue;
        }
        out.push(record);
    }
    for (const record of old.values()) {
        foreignGone(record, true);
    }
    return out;
}

function ForeignPortal({ children, container }) {
    const instance = current;
    const kept = useRef(null);
    if (kept.current !== null && kept.current.container !== container) {
        for (const record of kept.current.records) {
            foreignGone(record, true);
        }
        kept.current = null;
    }
    kept.current ??= { container, records: [], slot: instance.slot };
    const state = kept.current;
    const doc = container.nodeType === 9 || container.nodeType === 11 ? container : container.ownerDocument;
    state.records = foreignList(state, instance, state.records, children, doc);
    foreignOrder(container, foreignNodes(state.records));
    useLayoutEffect(() => () => {
        for (const record of kept.current?.records ?? []) {
            foreignGone(record, true);
        }
        kept.current = null;
    }, []);
    return null;
}

/// Whether a portal's container is a fragment of the page's own document, as Radix's Select renders its closed list's items into
/// to know what they say: see `FragmentPortal`.
function pageFragment(container) {
    return container !== null && typeof container === 'object' && container.nodeType === 11 && container.__live === undefined
        && (container.ownerDocument === undefined || container.ownerDocument === globalThis.document);
}

/// What's in a portal into a fragment that isn't in the page: it runs, as React renders it into the fragment, but isn't shown and
/// takes no room, as `<template>`'s content doesn't. Drawn where the portal's written, a closed Select's items took room under it.
/// Its key says what it is to `rsui/web`, whose DOM has what's in it in no element of the page, as it's in the fragment.
function FragmentPortal({ children }) {
    // The key that says so is on a group inside, as what a component draws next in its place takes the key of what it drew
    // before (see `update` in rsui-runtime), and Radix Select draws its open list where this was.
    return element('group', { overlay: 'fill', inert: true, opacity: 0 }, element('group', { key: '__fragment', display: 'contents' }, children));
}

/// How many portals into the page's `body` have opened, which is the order they're in there.
let portalsOpened = 0;
/// Whether a portal has opened in what's running now, which the rest that open before it's finished open with.
let portalOpening = false;

/// A portal into the page's `body`. What's in it is drawn where it's written, with what's around it there, as React has it, in a
/// group with no box of its own that says it's a portal, which `rsui/web` puts at the end of `body` for what the DOM says. It says
/// when it opened too, as each goes at the end of `body` then, so what opened later is drawn over what opened before, and gets
/// the pointer first, as on a page (see `overlay_rank` in rsui-layout).
function PagePortal({ children }) {
    const opened = useRef(0);
    if (opened.current === 0) {
        // Those that open in the same update are in `body` in the order they're written, as React puts them there as it commits,
        // whichever was drawn first: Radix's Dialog draws its content's portal before its overlay's when both open at once. So
        // they're all said to have opened together, and what's written later is over what's written before.
        if (!portalOpening) {
            portalOpening = true;
            portalsOpened += 1;
            queueMicrotask(() => {
                portalOpening = false;
            });
        }
        opened.current = portalsOpened;
    }
    // What's in it is in `body` for CSS too, as far as `rsui/web` has it (`__portalContent`): not in a box that holds what's fixed.
    const Inside = module.exports.__portalContent;
    const content = typeof Inside === 'function' ? __jsx(Inside, { children }, undefined, false) : children;
    return __jsx('group', { key: '__portal', display: 'contents', portalOrder: opened.current }, [content], true);
}

/// Whether a portal's container is another document's, rather than the page, which what's in a portal shows where it's written in.
function foreignContainer(container) {
    if (container === null || typeof container !== 'object' || container.__live !== undefined) {
        return false;
    }
    const doc = container.nodeType === 9 || container.nodeType === 11 ? container : container.ownerDocument;
    return doc !== null && doc !== undefined && typeof doc.createElement === 'function' && doc !== globalThis.document;
}

/// The next hook in the order they're called in, or nothing if it has to be added with `addHook`. Making a hook is
/// apart from finding one so that a render doesn't make a function it won't use for every hook.
function nextHook(kind) {
    const instance = current;
    if (instance === null) {
        throw new Error('Hooks can only be called while a component is rendering');
    }
    const at = hookAt++;
    const hooks = instance.hooks;
    if (at < hooks.length) {
        const hook = hooks[at];
        if (tracing && hook.site === undefined) {
            hook.site = host.hookSite();
        }
        // An effect's kind is when it runs, which can change from one render to the next, as it's an effect either way.
        if (hook.kind !== kind && (typeof kind !== 'number' || typeof hook.kind !== 'number')) {
            if (reloading) {
                throw RESTART;
            }
            throw new Error('A component has to call the same hooks on every render: hook number '
                + at + ' was ' + kindName(hook.kind) + ' and is now ' + kindName(kind));
        }
        return hook;
    }
    if ((instance.flags & RENDERED) !== 0) {
        if (reloading) {
            throw RESTART;
        }
        throw new Error('A component has to call the same hooks on every render: hook number '
            + at + ' is ' + kindName(kind) + ', which it didn\'t have before');
    }
    return undefined;
}

/// What a kind of hook is called. An effect's kind is the number of when it runs: `INSERTION`, `LAYOUT` or `PASSIVE`.
const kindName = (kind) => (typeof kind === 'number' ? 'effect' : kind);

/// Adds a hook, which says what kind it is itself, as its `kind` is one of the four properties an object has room for in itself.
function addHook(kind, hook) {
    const instance = current;
    if (tracing) {
        hook.site = host.hookSite();
    }
    if (instance.hooks === NO_HOOKS) {
        instance.hooks = [hook];
    } else {
        instance.hooks.push(hook);
    }
    return hook;
}

// What an effect or a memo keeps when it's given no dependencies, rather than each of the empty lists it's given.
const NO_DEPS = Object.freeze([]);
// A function kept as a dependency is kept as itself, so the engine never makes it do what a new one does (see `reads.rs` in
// rsui-js): an effect that depends on it runs again when it's a new one, as React has it.
const keptDeps = (deps) => {
    if (deps === undefined || deps.length === 0) {
        return deps?.length === 0 ? NO_DEPS : deps;
    }
    for (let at = 0; at < deps.length; at++) {
        if (typeof deps[at] === 'function') {
            host.keepAsItself(deps[at]);
        }
    }
    return deps;
};

function depsChanged(old, deps) {
    if (old === undefined || deps === undefined || old.length !== deps.length) {
        return true;
    }
    for (let at = 0; at < deps.length; at++) {
        if (!Object.is(old[at], deps[at])) {
            return true;
        }
    }
    return false;
}

/// Has a component render again, whatever's changed.
function markDirty(instance) {
    if ((instance.flags & DEAD) === 0) {
        instance.flags |= FORCED;
        if (instance.placedIn !== undefined) {
            placedChanged?.(instance);
            return;
        }
        host.dirty(instance.slot);
    }
}

/// Has a component render again because its state's been set, which needn't call it if that's all been set back by then.
function markStateSet(instance) {
    if ((instance.flags & DEAD) === 0) {
        instance.flags |= STATE_SET;
        host.dirty(instance.slot);
    }
}

// What a state's setter does, which isn't in the setter itself, as each variable a function uses from around it takes room in it.
function setState(made, instance, next) {
    const value = typeof next === 'function' ? next(made.value) : next;
    if (!Object.is(value, made.value)) {
        if (tracing && (instance.flags & DEAD) === 0) {
            host.noteSet(instance.slot, instance.hooks.indexOf(made), made.site, made.value, value, 'state', undefined, instance);
        }
        made.value = value;
        markStateSet(instance);
    }
}

function makeState(instance, initial) {
    const made = {
        value: typeof initial === 'function' ? initial() : initial,
        shown: undefined,
        set(next) {
            setState(made, instance, next);
        },
        kind: 'state',
    };
    return made;
}

function useState(initial) {
    const hook = nextHook('state') ?? addHook('state', makeState(current, initial));
    hook.shown = hook.value;
    return [hook.value, hook.set];
}

function makeReducer(instance, reducer, initial, init) {
    const made = {
        reducer,
        value: init ? init(initial) : initial,
        shown: undefined,
        dispatch(action) {
            dispatchTo(made, instance, action);
        },
        kind: 'reducer',
    };
    return made;
}

function dispatchTo(made, instance, action) {
    const value = made.reducer(made.value, action);
    if (!Object.is(value, made.value)) {
        if (tracing && (instance.flags & DEAD) === 0) {
            host.noteSet(instance.slot, instance.hooks.indexOf(made), made.site, made.value, value, 'reducer', action, instance);
        }
        made.value = value;
        markStateSet(instance);
    }
}

function useReducer(reducer, initial, init) {
    const hook = nextHook('reducer') ?? addHook('reducer', makeReducer(current, reducer, initial, init));
    hook.reducer = reducer;
    hook.shown = hook.value;
    return [hook.value, hook.dispatch];
}

function useRef(initial) {
    return nextHook('ref') ?? addHook('ref', { current: initial, kind: 'ref' });
}

const MEMO_CACHE_EMPTY = Symbol.for('react.memo_cache_sentinel');

function clearCompilerCache(instance) {
    if ((instance.flags & COMPILER_CACHE) !== 0) {
        instance.more.compilerCache = null;
        instance.flags &= ~COMPILER_CACHE;
    }
}

// React Compiler's caches have their own cursor: adding a compiled custom hook
// during reload must not move any of the component's ordinary state hooks.
function useMemoCache(size) {
    if (current === null) {
        throw new Error('Compiler memo caches can only be used while a component is rendering');
    }
    if (!Number.isInteger(size) || size < 0) {
        throw new Error('A compiler memo cache needs a nonnegative integer size');
    }
    let cache;
    if ((current.flags & COMPILER_CACHE) === 0) {
        cache = { data: [], index: 0 };
        moreOf(current).compilerCache = cache;
        current.flags |= COMPILER_CACHE;
    } else {
        cache = current.more.compilerCache;
    }
    const at = cache.index++;
    let data = cache.data[at];
    if (data === undefined) {
        data = new Array(size).fill(MEMO_CACHE_EMPTY);
        data[MEMO_CACHE_EMPTY] = true;
        cache.data[at] = data;
    } else if (data.length !== size) {
        throw new Error('A compiler memo cache must keep the same size between renders');
    }
    return data;
}

function useMemo(compute, deps) {
    const hook = nextHook('memo') ?? addHook('memo', { deps: undefined, value: undefined, kind: 'memo' });
    if (depsChanged(hook.deps, deps)) {
        hook.value = compute();
        hook.deps = keptDeps(deps);
    }
    return hook.value;
}

function useCallback(callback, deps) {
    const hook = nextHook('memo') ?? addHook('memo', { deps: undefined, value: undefined, kind: 'memo' });
    if (depsChanged(hook.deps, deps)) {
        hook.value = callback;
        hook.deps = keptDeps(deps);
    }
    return hook.value;
}

function useEffect(run, deps) {
    effect(run, deps, reloading, PASSIVE);
}

/// An effect that runs before any `useEffect` of the same commit, and before what changed is drawn, as what measures what it's
/// rendered does. What's inside runs before what's around it, and refs are given their elements along with these.
function useLayoutEffect(run, deps) {
    effect(run, deps, reloading, LAYOUT);
}

/// An effect that runs before any layout effect or ref of the same commit, as CSS-in-JS adds the rules what renders needs with.
/// An effect that runs as the component renders, before anything after it does, as CSS-in-JS inserts its rules with: so that
/// what's drawn with a class it's made has that class's CSS the first time, rather than being drawn without it and then again
/// with it, which on a page here can change what's around it, and make that start again, as react-select's field would every
/// time it was typed in. It's what emotion does where there's no `useInsertionEffect`. What it gives back is called before it
/// runs again, and when the component goes, as React has it.
function useInsertionEffect(run, deps) {
    // It's never run while it has no dependencies, which is how the first time is told, as with the others.
    const hook = nextHook(INSERTION) ?? addHook(INSERTION, { deps: undefined, run: null, cleanup: null, kind: INSERTION });
    if (reloading || depsChanged(hook.deps, deps)) {
        hook.deps = keptDeps(deps);
        const undo = hook.cleanup;
        hook.cleanup = null;
        undo?.();
        const result = run();
        hook.cleanup = typeof result === 'function' ? result : null;
    }
}

/// An effect that a new version of the code doesn't run again, for what's holding something that shouldn't go away because
/// a file was saved, like a window.
function useLastingEffect(run, deps) {
    effect(run, deps, false, PASSIVE);
}

function effect(run, deps, always, phase) {
    const instance = current;
    // An effect's kind is when it runs. It's never run while it has no dependencies, which is how the first time is told.
    const hook = nextHook(phase) ?? addHook(phase, { deps: undefined, run: null, cleanup: null, kind: phase });
    if (always || depsChanged(hook.deps, deps)) {
        hook.run = run;
        hook.deps = keptDeps(deps);
        hook.kind = phase;
        if (instance.activity !== null) {
            // What runs it again when the `Activity` it's in is shown after being hidden.
            hook.again = run;
        }
        phasesDue |= 1 << phase;
        if (instance.due === null) {
            instance.due = [];
        }
        // This has to be every time, as effects that were waiting can be dropped when they never settle.
        host.effects();
        instance.due.push(hook);
    }
}

function animatedAt(hook, at) {
    const elapsed = at - hook.started;
    if (elapsed >= hook.took) {
        return hook.to;
    }
    const left = 1 - elapsed / hook.took;
    return hook.from + (hook.to - hook.from) * (1 - left * left * left);
}

/// A number that moves smoothly to `target`, starting quickly and slowing down to arrive. It costs nothing once it's there.
function useAnimated(target, duration = 250) {
    const hook = nextHook('animated') ?? addHook('animated', { from: target, to: target, started: 0, took: 0, kind: 'animated' });
    if (!host.animationsOn() || !Number.isFinite(target)) {
        hook.from = hook.to = target;
        hook.took = 0;
        return target;
    }
    const now = host.now();
    if (hook.to !== target) {
        hook.from = animatedAt(hook, now);
        hook.to = target;
        hook.started = now;
        hook.took = duration;
    }
    const value = animatedAt(hook, now);
    if (value !== hook.to) {
        // It renders again on the next frame to move, which isn't because of its state.
        current.flags |= FORCED;
        host.frame();
    } else {
        hook.took = 0;
    }
    return value;
}

/// What a context's `Provider` gives what's in it, in the list an instance's `provides` starts. What reads it is in `watchers`: see
/// `watch`. These have no more than four properties, as each one after that takes room of its own.
class Provided {
    constructor(context, value, next) {
        this.context = context;
        this.value = value;
        this.watchers = null;
        this.next = next;
    }
}
Provided.prototype.here = false;

/// The same, for what a component passes down with `provideHere`, which is for what it renders and not for itself.
class PassedHere {
    constructor(context, value, next) {
        this.context = context;
        this.value = value;
        this.watchers = null;
        this.next = next;
    }
}
PassedHere.prototype.here = true;

// What reads a provider is nothing, the one instance that does, a list of a few, or a `Set` once there are more than this, as most
// have none or one, and a `Set` of one takes four times what the object it's for does.
const FEW_WATCHERS = 8;

/// Has `instance` read `provider`, saying whether it wasn't reading it already.
function watch(provider, instance) {
    const watchers = provider.watchers;
    if (watchers === null) {
        provider.watchers = instance;
        return true;
    }
    if (watchers === instance) {
        return false;
    }
    if (watchers instanceof Set) {
        if (watchers.has(instance)) {
            return false;
        }
        watchers.add(instance);
        return true;
    }
    if (Array.isArray(watchers)) {
        if (watchers.includes(instance)) {
            return false;
        }
        if (watchers.length < FEW_WATCHERS) {
            watchers.push(instance);
        } else {
            const many = new Set(watchers);
            many.add(instance);
            provider.watchers = many;
        }
        return true;
    }
    provider.watchers = [watchers, instance];
    return true;
}

function unwatch(provider, instance) {
    const watchers = provider.watchers;
    if (watchers === instance) {
        provider.watchers = null;
    } else if (watchers instanceof Set) {
        watchers.delete(instance);
    } else if (Array.isArray(watchers)) {
        const at = watchers.indexOf(instance);
        if (at >= 0) {
            watchers.splice(at, 1);
            if (watchers.length === 1) {
                provider.watchers = watchers[0];
            }
        }
    }
}

/// What reads `provider`, in the order they started to, to go through.
function watchersOf(provider) {
    const watchers = provider.watchers;
    if (watchers === null) {
        return NO_HOOKS;
    }
    return watchers instanceof Set || Array.isArray(watchers) ? watchers : [watchers];
}

function createContext(fallback) {
    const context = { fallback };
    context.Provider = function Provider(props) {
        const instance = current;
        if (instance.provides === null) {
            instance.provides = new Provided(context, props.value, null);
        } else if (instance.provides.context !== context) {
            // It's a new version of the file this was made in, and what reads it is reading the new one.
            instance.provides.context = context;
        }
        if (!Object.is(instance.provides.value, props.value)) {
            instance.provides.value = props.value;
            for (const watcher of watchersOf(instance.provides)) {
                if (tracing && (watcher.flags & DEAD) === 0) {
                    host.noteContext(watcher.slot, watcher.more?.contextSites?.get(instance.provides));
                }
                markDirty(watcher);
            }
        }
        return props.children;
    };
    // What says it's this context's, for `use`.
    context.Provider.context = context;
    return context;
}

/// What a provider above gives, or the context's fallback, as `use` reads it. It isn't a hook, as React's isn't: it takes no place
/// among them, so it can be called in an `if`, as code written for React does.
function useContext(context) {
    const instance = current;
    if (instance === null) {
        throw new Error('Hooks can only be called while a component is rendering');
    }
    return readContext(instance, context);
}

/// What a context is for what's rendering now, or what it is when nothing gives it, which isn't a hook, so it can be asked from
/// anywhere: during a render it's what's above that, and otherwise it's the fallback.
function contextNow(context) {
    for (let at = current; at !== null; at = at.parent) {
        for (let provider = at.provides; provider !== null; provider = provider.next) {
            // What a component passes down with `provideHere` is for what it renders, not for itself.
            if (provider.context === context && (at !== current || !provider.here)) {
                return provider.value;
            }
        }
    }
    return context.fallback;
}

/// Passes a context's value down to everything the component rendering now renders, as a provider around what it gives back
/// would, without one: so that whether there's anything to pass down doesn't change what's there, which would make all of it
/// start again. It's to be called on every render from the first, even when it's only passing on what's above, as what reads it
/// keeps reading the same place from then on. It isn't a hook, and the component itself still reads what's above it.
function provideHere(context, value) {
    const instance = current;
    let provider = instance.provides;
    while (provider !== null && provider.context !== context) {
        provider = provider.next;
    }
    if (provider === null) {
        instance.provides = new PassedHere(context, value, instance.provides);
        return;
    }
    if (!Object.is(provider.value, value)) {
        const before = provider.value;
        provider.value = value;
        toldChanged(provider, before, value);
    }
}

/// Has what reads `provider` render again now that it's `after` rather than `before`. A context that says how (`passedOn`) has only
/// what read some part of it that's different render again, as a page's elements are only styled again for what they use: what
/// didn't is left as it is, and what it passes down to what's in it is worked out here and passed on in turn. That's how a popover
/// placed again on each frame of scrolling, whose element sets variables saying how much room it has, doesn't have everything in
/// it render again with it.
function toldChanged(provider, before, after) {
    const passedOn = provider.context.passedOn;
    for (const watcher of watchersOf(provider)) {
        if ((watcher.flags & DEAD) !== 0) {
            continue;
        }
        const note = passedOn === undefined || watcher.reads === null ? undefined : watcher.reads.get(provider.context);
        if (note !== undefined && note !== null) {
            let own = watcher.provides;
            while (own !== null && !(own.context === provider.context && own.here === true)) {
                own = own.next;
            }
            const down = passedOn(note, before, after, own === null ? undefined : own.value);
            if (down !== undefined) {
                if (own !== null && !Object.is(own.value, down)) {
                    const was = own.value;
                    own.value = down;
                    toldChanged(own, was, down);
                }
                continue;
            }
        }
        if (tracing) {
            host.noteContext(watcher.slot, watcher.more?.contextSites?.get(provider));
        }
        markDirty(watcher);
    }
}

/// Says what the component rendering now read of a context that has `passedOn`, for [`toldChanged`]: what `passedOn` is given to
/// say whether something it read is different. `null` says it can't be known, and it renders again whenever it changes.
function readsHere(context, note) {
    const instance = current;
    if (instance.reads === null) {
        if (note === null) {
            return;
        }
        instance.reads = new Map();
    }
    instance.reads.set(context, note);
}

const isThenable = (value) => value !== null && (typeof value === 'object' || typeof value === 'function') && typeof value.then === 'function';

/// Whether what a component gave back is only text, as a string, or a list or fragment of strings with nothing else in it but
/// nothing, as Ant Design's Typography gives back its words and the buttons it hasn't got.
function onlyText(output, depth) {
    if (typeof output === 'string' || typeof output === 'number') {
        return depth > 0 || output !== '';
    }
    if (depth > 8) {
        return false;
    }
    if (Array.isArray(output)) {
        let some = false;
        for (const one of output) {
            if (one === null || one === undefined || one === false || one === true || one === '' || (Array.isArray(one) && nothingIn(one))) {
                continue;
            }
            if (!onlyText(one, depth + 1)) {
                return false;
            }
            some = true;
        }
        return some;
    }
    if (output !== null && typeof output === 'object' && output.type === Fragment) {
        return onlyText(output.kids ?? output.props?.children, depth + 1);
    }
    return false;
}

/// Whether a list has nothing in it that's drawn, as a list of buttons that aren't there.
const nothingIn = (list) => list.every((one) => one === null || one === undefined || one === false || one === true || one === '' || (Array.isArray(one) && nothingIn(one)));

/// The `Suspense` that shows something else while what's below `instance` waits. What's shown while one waits waits for the one
/// around that, so each fallback that's passed on the way up skips one.
function suspenseAbove(instance) {
    let skip = 0;
    for (let at = instance.parent; at !== null; at = at.parent) {
        if ((at.flags & FALLBACK) !== 0) {
            skip += 1;
        } else if (at.more !== null && at.more.suspense !== null) {
            if (skip === 0) {
                return at;
            }
            skip -= 1;
        }
    }
    return null;
}

/// Notes on a promise how it went, as React does, so that what reads it once it's settled has what it gave straight away. Code
/// that made it can set these itself, as a library that knows already does.
function track(thenable) {
    if (typeof thenable.status === 'string') {
        return;
    }
    thenable.status = 'pending';
    thenable.then(
        (value) => {
            if (thenable.status === 'pending') {
                thenable.status = 'fulfilled';
                thenable.value = value;
            }
        },
        (reason) => {
            if (thenable.status === 'pending') {
                thenable.status = 'rejected';
                thenable.reason = reason;
            }
        },
    );
}

/// What a component that's waiting for `thenable` does: the `Suspense` around it shows its fallback until it's settled, and then
/// what's in it renders again. With none around it, it's only this that shows nothing until then.
function suspend(instance, thenable) {
    track(thenable);
    const boundary = suspenseAbove(instance);
    if (boundary === null) {
        const more = moreOf(instance);
        if (more.waitingFor !== thenable) {
            more.waitingFor = thenable;
            const settled = () => {
                if (more.waitingFor === thenable) {
                    more.waitingFor = undefined;
                    markDirty(instance);
                }
            };
            thenable.then(settled, settled);
        }
        return;
    }
    const waiting = boundary.more.suspense.waiting;
    if (waiting.has(thenable)) {
        return;
    }
    waiting.add(thenable);
    if (waiting.size === 1) {
        markDirty(boundary);
    }
    const settled = () => {
        waiting.delete(thenable);
        if (waiting.size === 0) {
            markDirty(boundary);
        }
    };
    thenable.then(settled, settled);
}

/// What a `Suspense` shows while it waits, which is only told apart from what it shows otherwise, for `suspenseAbove`.
function SuspenseFallback(props) {
    current.flags |= FALLBACK;
    return props.children;
}

/// Shows `fallback` while anything in it is waiting, as `use` does for a promise that hasn't settled, and what's in it once nothing
/// is. What was in it starts again then, as there's nothing to keep it in while it's not shown.
function Suspense(props) {
    const instance = current;
    const hook = nextHook('suspense') ?? addHook('suspense', { waiting: new Set(), kind: 'suspense' });
    moreOf(instance).suspense = hook;
    if (hook.waiting.size > 0) {
        return __jsx(SuspenseFallback, NO_PROPS, props.fallback === undefined ? undefined : [props.fallback], false);
    }
    return props.children;
}

/// Whether what's in an `Activity` is hidden, by it or by one that it's in.
function isHidden(activity) {
    for (let at = activity; at !== null; at = at.above) {
        if (at.hidden) {
            return true;
        }
    }
    return false;
}

/// Each of what's in an `Activity`, in those in it too, with what's inside before what it's in when `inward` is false.
function eachInActivity(activity, inward, visit) {
    const members = [...activity.members];
    if (!inward) {
        members.reverse();
    }
    for (const member of members) {
        if (inward) {
            visit(member);
        }
        const below = activityBelow(member);
        if (below !== activity && below !== null) {
            eachInActivity(below, inward, visit);
        }
        if (!inward) {
            visit(member);
        }
    }
}

/// Undoes the layout and passive effects of what's in an `Activity` that's just been hidden, keeping them to run again.
function hideActivity(activity) {
    let uncaught = NO_ERROR;
    eachInActivity(activity, true, (member) => {
        if ((member.flags & DEAD) !== 0) {
            return;
        }
        for (const hook of member.hooks) {
            if (typeof hook.kind === 'number' && hook.kind !== INSERTION) {
                if (hook.cleanup) {
                    const cleanup = hook.cleanup;
                    hook.cleanup = null;
                    try {
                        cleanup();
                    } catch (error) {
                        uncaught = firstUncaught(uncaught, caughtOutside(member, error));
                    }
                }
                hook.run ??= hook.again ?? null;
            }
        }
    });
    if (uncaught !== NO_ERROR) {
        throw uncaught;
    }
}

/// Runs the effects of what's in an `Activity` that's just been shown, its layout effects and then its passive ones, what's inside
/// before what it's in.
function showActivity(activity) {
    let uncaught = NO_ERROR;
    for (const phase of [LAYOUT, PASSIVE]) {
        eachInActivity(activity, false, (member) => {
            if ((member.flags & DEAD) !== 0 || member.hooks === NO_HOOKS) {
                return;
            }
            const due = member.due;
            member.due = member.hooks;
            uncaught = runEffects(member, phase, uncaught);
            member.due = due;
        });
    }
    if (uncaught !== NO_ERROR) {
        throw uncaught;
    }
}

const SHOWN_ACTIVITY = Object.freeze({ display: 'contents' });
const HIDDEN_ACTIVITY = Object.freeze({ position: 'absolute', width: 0, height: 0, opacity: 0, inert: true });

/// What's in it, while its `mode` is `"visible"`, as it is unless it says. While it's `"hidden"`, what's in it is kept, with its
/// state, though it isn't drawn and nothing can reach it, and its effects are undone until it's shown again, as React 19.2's is.
function Activity(props) {
    const instance = current;
    const hidden = props.mode === 'hidden';
    const hook = nextHook('activity') ?? addHook('activity', { hidden, shownAs: hidden, members: new Set(), above: instance.activity, kind: 'activity' });
    moreOf(instance).below = hook;
    // What's in it renders knowing, so that what it has to run waits while it's hidden.
    hook.hidden = hidden;
    effect(() => {
        if (hook.shownAs !== hidden) {
            hook.shownAs = hidden;
            if (hidden) {
                hideActivity(hook);
            } else {
                showActivity(hook);
            }
        }
    }, [hidden], false, LAYOUT);
    return __jsx('group', hidden ? HIDDEN_ACTIVITY : SHOWN_ACTIVITY, props.children === undefined ? undefined : [props.children], false);
}

/// Reads what a provider above gives, as `useContext` does, though it needn't be called in the same place on every render.
function readContext(instance, context) {
    // What it's read before is what it reads now, as what's above something doesn't change.
    const watching = instance.watching;
    if (watching !== null) {
        for (let index = 0; index < watching.length; index++) {
            if (watching[index].context === context) {
                return watching[index].value;
            }
        }
    }
    let provider = null;
    for (let at = instance.parent; at !== null && provider === null; at = at.parent) {
        for (let one = at.provides; one !== null; one = one.next) {
            if (one.context === context) {
                provider = one;
                break;
            }
        }
    }
    if (provider === null) {
        return context.fallback;
    }
    if (watch(provider, instance)) {
        (instance.watching ??= []).push(provider);
        // What stands for an element drawn in place isn't what a debugger says rendered: its owner is.
        if (tracing && instance.placedIn === undefined) {
            // Where it's read from, which names it, for saying why what reads it rendered.
            (moreOf(instance).contextSites ??= new Map()).set(provider, host.hookSite());
        }
    }
    return provider.value;
}

/// What a promise gave, waiting for it with the `Suspense` around this if it hasn't settled, or what a context has. Unlike a hook,
/// it can be called in an `if` or a loop. What a promise was rejected with is thrown, for an error boundary.
function use(usable) {
    const instance = current;
    if (instance === null) {
        throw new Error('use can only be called while a component is rendering');
    }
    if (isThenable(usable)) {
        track(usable);
        if (usable.status === 'fulfilled') {
            return usable.value;
        }
        if (usable.status === 'rejected') {
            throw usable.reason;
        }
        throw usable;
    }
    if (usable !== null && typeof usable === 'object' && usable.Provider?.context === usable) {
        return readContext(instance, usable);
    }
    throw new Error('An unsupported type was passed to use(): ' + String(usable));
}

/// Gives a ref what it refers to, and gives back what undoes that: what a callback ref gave back, if it gave back a function, as in
/// React 19, or giving it null.
function attachRef(ref, value) {
    if (typeof ref === 'function') {
        const cleanup = ref(value);
        return typeof cleanup === 'function' ? cleanup : () => ref(null);
    }
    if (ref !== null && ref !== undefined) {
        ref.current = value;
        return () => {
            ref.current = null;
        };
    }
    return undefined;
}

/// A box that's ticked or not, with what it's for beside it. It says what it is to agents and screen readers too.
function Checkbox({ id, checked, label, onChange, ...rest }) {
    const tick = element('group', {
        width: 16,
        height: 16,
        radius: 4,
        align: 'center',
        justify: 'center',
        background: checked ? '#1f6feb' : widgets.field,
        borderColor: checked ? '#1f6feb' : widgets.edge,
        borderWidth: 1,
    }, checked ? element('text', { color: '#ffffff', fontSize: 11, fontWeight: 'bold' }, '\u2713') : null);
    return element('checkbox', {
        key: id,
        name: label,
        checked,
        direction: 'row',
        align: 'center',
        gap: 8,
        ...rest,
        onToggle: onChange === undefined ? undefined : () => onChange(!checked),
    }, tick, rest.showLabel === false ? null : element('text', { color: rest.color }, label));
}

// Shapes. What's inside an `Svg` only says what to draw, so it's all read from here, into one element, however many there
// are. They don't render anything themselves.
const Path = () => null;
const Line = () => null;
const Polyline = () => null;
const Polygon = () => null;
const Rect = () => null;
const Circle = () => null;
const Ellipse = () => null;
const G = () => null;

/// What's a shape, which an `Svg` reads rather than renders.
const SHAPE_KINDS = new Set([Path, Line, Polyline, Polygon, Rect, Circle, Ellipse, G]);
/// The shape each SVG tag is, for a `<circle>` that's still its tag, as an app's own JSX makes it without React's runtime.
const SHAPE_TAGS = new Map([['path', Path], ['line', Line], ['polyline', Polyline], ['polygon', Polygon], ['rect', Rect], ['circle', Circle], ['ellipse', Ellipse], ['g', G]]);

/// Whether something in an `Svg` is a component of the app's or a library's, which renders, rather than a shape, an element of
/// the page or a fragment, which are read as they are.
function svgComponent(type) {
    return typeof type === 'function' && type !== Fragment && !SHAPE_KINDS.has(type) && type.__shape === undefined && type.__webTag === undefined;
}

/// Whether anything in what's in an `Svg` is a component that renders, which most pictures, as icons, have none of.
function svgHasComponents(children) {
    if (Array.isArray(children)) {
        return children.some(svgHasComponents);
    }
    if (children === null || typeof children !== 'object') {
        return false;
    }
    return svgComponent(children.type) || svgHasComponents(children.kids ?? children.props?.children);
}

function svgGone(record) {
    for (const child of record.children) {
        svgGone(child);
    }
    if (record.instance !== undefined) {
        unmount(record.instance);
    }
}

/// What's in an `Svg`, with each component in it rendered as what it gives back, so that what a library's components make, as
/// recharts' axes and lines are, is drawn too. Shapes and elements of the page stay as they are, with what's inside them gone
/// through the same way. The components keep their hooks from one render to the next, as anywhere, and render again with the
/// `Svg`, which is what Rust knows about.
function svgExpanded(state, owner, before, children) {
    const old = new Map();
    for (const record of before) {
        old.set(record.key, record);
    }
    const elements = [];
    const records = [];
    for (const { key, element } of foreignItems(children, '', [])) {
        if (element === null || typeof element !== 'object') {
            elements.push(element);
            continue;
        }
        const type = element.type;
        let record = old.get(key);
        if (record !== undefined && record.kind !== type) {
            svgGone(record);
            record = undefined;
        }
        old.delete(key);
        if (svgComponent(type)) {
            const props = element.props ?? NO_PROPS;
            record ??= { key, kind: type, instance: newInstance(state.slot, owner), children: [] };
            const instance = record.instance;
            const output = (instance.flags & (RENDERED | FORCED | STATE_SET)) !== RENDERED || instance.props !== props || instance.everything !== everything
                ? render(instance, type, props)
                : instance.output;
            const inside = svgExpanded(state, instance, record.children, output);
            record.children = inside.records;
            elements.push(...inside.elements);
            flushEffects(instance);
        } else {
            record ??= { key, kind: type, children: [] };
            const given = element.kids ?? element.props?.children;
            if (svgHasComponents(given) || record.children.length > 0) {
                const inside = svgExpanded(state, owner, record.children, given);
                record.children = inside.records;
                elements.push(element.kids !== null && element.kids !== undefined
                    ? { ...element, kids: inside.elements }
                    : { ...element, props: { ...element.props, children: inside.elements } });
            } else {
                elements.push(element);
            }
        }
        records.push(record);
    }
    for (const record of old.values()) {
        svgGone(record);
    }
    return { elements, records };
}

function pathOf(type, props) {
    if (type === Path) {
        return props.d ?? '';
    }
    const n = (value) => Number(value ?? 0);
    if (type === Line) {
        return `M${n(props.x1)} ${n(props.y1)}L${n(props.x2)} ${n(props.y2)}`;
    }
    if (type === Polyline || type === Polygon) {
        const points = Array.isArray(props.points)
            ? props.points.map((point) => (Array.isArray(point) ? point.join(' ') : point)).join(' ')
            : String(props.points ?? '');
        return points === '' ? '' : `M${points}${type === Polygon ? 'Z' : ''}`;
    }
    if (type === Rect) {
        const [x, y, width, height] = [n(props.x), n(props.y), n(props.width), n(props.height)];
        // Its corners' radii across and down, either of which is the other when only one's said, as SVG has them.
        const saidX = props.rx ?? props.radius ?? props.ry;
        const saidY = props.ry ?? props.rx ?? props.radius;
        const rx = Math.min(n(saidX), width / 2);
        const ry = Math.min(n(saidY), height / 2);
        if (!(rx > 0 && ry > 0)) {
            return `M${x} ${y}h${width}v${height}h${-width}Z`;
        }
        const r = `a${rx} ${ry} 0 0 1`;
        const [across, down] = [width - 2 * rx, height - 2 * ry];
        return `M${x + rx} ${y}h${across}${r} ${rx} ${ry}v${down}${r} ${-rx} ${ry}h${-across}${r} ${-rx} ${-ry}v${-down}${r} ${rx} ${-ry}Z`;
    }
    if (type === Circle || type === Ellipse) {
        const [cx, cy] = [n(props.cx), n(props.cy)];
        const [rx, ry] = type === Circle ? [n(props.r), n(props.r)] : [n(props.rx), n(props.ry)];
        // From the right, going clockwise, as SVG has it, which is where a dashed one's dashes start.
        return `M${cx + rx} ${cy}a${rx} ${ry} 0 1 1 ${-2 * rx} 0a${rx} ${ry} 0 1 1 ${2 * rx} 0Z`;
    }
    return null;
}

const colorText = (color) => {
    if (color === undefined || color === null || color === 'none') {
        return '';
    }
    if (typeof color === 'string' && color.toLowerCase() === 'currentcolor') {
        return 'currentColor';
    }
    if (typeof color === 'number') {
        return '#' + color.toString(16).padStart(6, '0');
    }
    // A color as CSS writes it, as `tomato` or `rgb(0 0 0)`, if `rsui/web` is there to say what it is.
    return globalThis.__rsuiColor?.(String(color)) ?? String(color);
};

/// A dash pattern as SVG's `stroke-dasharray` says it, as numbers with commas between them, or nothing for a solid line.
function dashText(dashes) {
    if (dashes === undefined || dashes === null || dashes === 'none') {
        return '';
    }
    const lengths = (Array.isArray(dashes) ? dashes : String(dashes).replace(/px/g, '').trim().split(/[\s,]+/))
        .filter((length) => length !== '')
        .map(Number);
    return lengths.length === 0 || lengths.some((length) => !Number.isFinite(length) || length < 0) ? '' : lengths.join(',');
}

function addLayers(kids, around, layers) {
    if (Array.isArray(kids)) {
        for (const kid of kids) {
            addLayers(kid, around, layers);
        }
        return;
    }
    if (kids === null || typeof kids !== 'object') {
        return;
    }
    const props = kids.props ?? NO_PROPS;
    // What's in it, which an element written in lower case, as a page's `<g>` is, keeps by itself rather than in its props.
    const inside = kids.kids ?? props.children;
    if (kids.type === Fragment) {
        addLayers(inside, around, layers);
        return;
    }
    // What's in the picture, as a page has it as elements, for what asks about them, which `around.found` is told.
    const tag = typeof kids.type === 'string' ? kids.type : kids.type?.__webTag ?? null;
    // What CSS says about this part, by its id, its classes, its attributes and its own `style`, goes over what it says itself, as
    // it does on a page: Mantine's ring is `<circle class="m_b1ca1fbf">`, whose stroke is only said in its stylesheet.
    // And what it says with `var()`, as shadcn's chart's `fill="var(--color-desktop)"` does, which is worked out there too.
    const part = around.parts === undefined || (props.id === undefined && !saysVariable(props) && (tag === null || (props.className === undefined && props.style === undefined)))
        ? NO_PROPS : around.parts(props.id, tag, props);
    if (part.__shapeMoves !== undefined) {
        around.moves?.push(part.__shapeMoves);
    }
    if (props.__shapeMoves !== undefined) {
        around.moves?.push(props.__shapeMoves);
    }
    const own = {
        parts: around.parts,
        moves: around.moves,
        fill: part.fill ?? props.fill ?? around.fill,
        stroke: part.stroke ?? props.stroke ?? around.stroke,
        strokeWidth: part.strokeWidth ?? props.strokeWidth ?? around.strokeWidth,
        strokeLinecap: part.strokeLinecap ?? props.strokeLinecap ?? around.strokeLinecap,
        strokeLinejoin: part.strokeLinejoin ?? props.strokeLinejoin ?? around.strokeLinejoin,
        strokeMiterlimit: part.strokeMiterlimit ?? props.strokeMiterlimit ?? around.strokeMiterlimit,
        fillRule: part.fillRule ?? props.fillRule ?? around.fillRule,
        strokeDasharray: part.strokeDasharray ?? props.strokeDasharray ?? around.strokeDasharray,
        strokeDashoffset: part.strokeDashoffset ?? props.strokeDashoffset ?? around.strokeDashoffset,
        textAnchor: part.textAnchor ?? props.textAnchor ?? around.textAnchor,
        // What's inside goes first, as what's around it applies to all of that.
        transform: props.transform === undefined ? around.transform : around.transform === undefined ? props.transform : around.transform + ' ' + props.transform,
    };
    // A component that says it's one of these, as what a page's `<path>` is made into does, is that.
    const named = kids.type?.__shape ?? kids.type;
    const shape = typeof named === 'string' ? SHAPE_TAGS.get(named) ?? named : named;
    const at = around.found === undefined || tag === null ? -1 : around.found.push({ tag, props, parent: around.foundAt ?? -1 }) - 1;
    own.found = around.found;
    own.foundAt = at >= 0 ? at : around.foundAt;
    // Its font, as CSS and its attributes say, passed down as SVG's text properties are.
    own.font = svgFont(around.font, part, props);
    if (shape === G || SVG_GROUPS.has(shape)) {
        addLayers(inside, own, layers);
        return;
    }
    // A `<tspan>` is drawn by the `<text>` it's in, and is only gone through here for what's in the picture.
    if (shape === 'text' || shape === 'tspan') {
        if (shape === 'text') {
            addText(props, own, layers);
        }
        if (at >= 0) {
            addLayers(inside, { ...own, found: around.found, foundAt: at }, []);
        }
        return;
    }
    const d = pathOf(shape, props);
    if (d === null) {
        if (typeof shape === 'string' || tag !== null) {
            // What isn't a shape, as `<defs>`, `<clipPath>` and `<text>` aren't, isn't drawn, as CSS that can't be done here isn't,
            // and it's said once. What's inside still counts as part of the page.
            if (!SVG_DEFINITIONS.has(shape) && !svgSaid.has(shape)) {
                svgSaid.add(shape);
                console.warn(`rsui: <${shape}> in an <svg> can't be drawn here, so it's left out`);
            }
            if (at >= 0) {
                addLayers(inside, { ...own, found: around.found, foundAt: at }, []);
            }
            return;
        }
        throw new TypeError('Only Path, Line, Polyline, Polygon, Rect, Circle, Ellipse and G can be in an Svg.');
    }
    const fill = own.fill === undefined ? 'currentColor' : colorText(own.fill);
    const offset = parseFloat(own.strokeDashoffset ?? 0);
    // How far a mitred corner reaches before it's bevelled, in widths of the line, which is 4 unless it says.
    const limit = parseFloat(own.strokeMiterlimit ?? 4);
    const line = [fill, colorText(own.stroke), own.strokeWidth ?? 1, own.strokeLinecap ?? 'butt', d, own.transform ?? '', dashText(own.strokeDasharray), Number.isFinite(offset) ? offset : 0, own.strokeLinejoin ?? 'miter', Number.isFinite(limit) && limit >= 1 ? limit : 4, own.fillRule === 'evenodd' ? 'evenodd' : 'nonzero'];
    layers.push(line.join('\t').replace(/\n/g, ' '));
}

/// Whether a shape's presentation attributes say something with `var()`, which only CSS can work out.
function saysVariable(props) {
    return (typeof props.fill === 'string' && props.fill.includes('var(')) || (typeof props.stroke === 'string' && props.stroke.includes('var('));
}

/// The font something in an `<svg>` is in: what's around it, then its attributes, then what CSS says about it.
function svgFont(around, part, props) {
    const pick = (name) => part[name] ?? props[name] ?? around?.[name];
    return { fontSize: pick('fontSize'), fontFamily: pick('fontFamily'), fontWeight: pick('fontWeight'), fontStyle: pick('fontStyle') };
}

/// A length in an `<svg>`'s text, as `x="12"`, `dy="0.71em"` or `dy="-4px"`, from its font's size for `em`. Only the first of a
/// list counts, as one per character isn't followed here.
function svgLength(value, size) {
    if (value === undefined || value === null || value === '') {
        return undefined;
    }
    if (typeof value === 'number') {
        return value;
    }
    const first = String(value).trim().split(/[\s,]+/)[0];
    const number = parseFloat(first);
    if (!Number.isFinite(number)) {
        return undefined;
    }
    return /em$/.test(first) ? number * size : number;
}

/// `<text>`, with what's in it and its `<tspan>`s, as layers filled with the outlines of its glyphs, set where SVG puts them: from
/// `x` and `y` on its baseline, moved by `dx` and `dy`, and lined up on `x` by `text-anchor` (`textAnchor`), in each run that
/// starts at an `x` of its own. Recharts draws a chart's axis labels this way.
function addText(props, own, layers) {
    const textPath = host?.canvas?.canvasTextPath;
    if (typeof textPath !== 'function') {
        return;
    }
    const pen = { x: 0, y: 0 };
    // Each run that starts at an `x` of its own, which `text-anchor` lines up by itself, and the pieces in it.
    const chunks = [];
    let chunk = null;
    const visit = (node, at, style, anchor) => {
        const size = Number(parseFloat(style.fontSize)) || 16;
        const x = svgLength(at.x, size);
        const y = svgLength(at.y, size);
        if (x !== undefined) {
            pen.x = x;
            chunk = null;
        }
        if (y !== undefined) {
            pen.y = y;
        }
        pen.x += svgLength(at.dx, size) ?? 0;
        pen.y += svgLength(at.dy, size) ?? 0;
        const kids = Array.isArray(node) ? node : [node];
        for (const kid of kids.flat(Infinity)) {
            if (kid === null || kid === undefined || kid === false || kid === true) {
                continue;
            }
            if (typeof kid === 'string' || typeof kid === 'number') {
                const text = String(kid).replace(/\s+/g, ' ');
                if (text.trim() === '') {
                    continue;
                }
                const weight = style.fontWeight === 'bold' ? 700 : style.fontWeight === 'normal' || style.fontWeight === undefined ? 400 : Number(style.fontWeight) || 400;
                const found = textPath(size, weight, style.fontStyle === 'italic' ? 1 : 0, String(style.fontFamily ?? 'sans-serif'), 0, text);
                if (!Array.isArray(found)) {
                    continue;
                }
                if (chunk === null) {
                    chunk = { anchor, width: 0, pieces: [] };
                    chunks.push(chunk);
                }
                chunk.pieces.push({ d: found[0], x: pen.x, y: pen.y, fill: style.fill, stroke: style.stroke, strokeWidth: style.strokeWidth, transform: style.transform });
                chunk.width += found[1];
                pen.x += found[1];
                continue;
            }
            if (typeof kid !== 'object') {
                continue;
            }
            const kidProps = kid.props ?? NO_PROPS;
            const tag = typeof kid.type === 'string' ? kid.type : kid.type?.__webTag ?? kid.type?.__shape ?? null;
            if (tag !== 'tspan' && kid.type !== Fragment) {
                continue;
            }
            const part = tag === null || own.parts === undefined || (kidProps.id === undefined && kidProps.className === undefined && kidProps.style === undefined)
                ? NO_PROPS : own.parts(kidProps.id, tag, kidProps);
            const inner = {
                ...style,
                ...svgFont(style, part, kidProps),
                fill: part.fill ?? kidProps.fill ?? style.fill,
                stroke: part.stroke ?? kidProps.stroke ?? style.stroke,
            };
            const kidAnchor = part.textAnchor ?? kidProps.textAnchor ?? anchor;
            visit(kid.kids ?? kidProps.children, kid.type === Fragment ? {} : kidProps, inner, kidAnchor);
        }
    };
    const style = { ...own.font, fill: own.fill, stroke: own.stroke, strokeWidth: own.strokeWidth, transform: own.transform };
    visit(props.children, props, style, own.textAnchor ?? 'start');
    for (const one of chunks) {
        const shift = one.anchor === 'middle' ? -one.width / 2 : one.anchor === 'end' ? -one.width : 0;
        for (const piece of one.pieces) {
            if (piece.d === '') {
                continue;
            }
            const fill = piece.fill === undefined ? 'currentColor' : colorText(piece.fill);
            const moved = `translate(${piece.x + shift} ${piece.y})`;
            const transform = piece.transform === undefined || piece.transform === '' ? moved : piece.transform + ' ' + moved;
            const line = [fill, colorText(piece.stroke), piece.strokeWidth ?? 1, 'butt', piece.d, transform, '', 0, 'miter'];
            layers.push(line.join('\t').replace(/\n/g, ' '));
        }
    }
}

/// What's drawn like a `<g>`, as what's inside them is drawn in their place.
const SVG_GROUPS = new Set(['a', 'switch']);
/// What says how something else is drawn, rather than being drawn itself, which is nothing to say anything about.
const SVG_DEFINITIONS = new Set(['defs', 'clipPath', 'mask', 'pattern', 'symbol', 'marker', 'linearGradient', 'radialGradient', 'stop', 'filter',
    'title', 'desc', 'metadata', 'style']);
const svgSaid = new Set();

/// A shape: `<Svg viewBox="0 0 24 24"><Path d="M2 2H22V22Z" /></Svg>`. `fill`, `stroke`, `strokeWidth` and `strokeLinecap` go
/// on what's inside or on this, for all of it. `currentColor`, which is what fills if nothing says, is this one's `color`.
function Svg({ viewBox, fill, fillRule, stroke, strokeWidth, strokeLinecap, strokeLinejoin, strokeMiterlimit, strokeDasharray, strokeDashoffset, preserveAspectRatio, __parts: parts, __elements: told, children, ...props }) {
    // The components in it render with it, keeping their hooks: see `svgExpanded`. A picture with none, as an icon is, is read
    // as it's written.
    const instance = current;
    const kept = useRef(null);
    useLayoutEffect(() => () => {
        for (const record of kept.current?.records ?? []) {
            svgGone(record);
        }
    }, []);
    if (kept.current !== null || svgHasComponents(children)) {
        kept.current ??= { slot: instance.slot, records: [] };
        const expanded = svgExpanded(kept.current, instance, kept.current.records, children);
        kept.current.records = expanded.records;
        children = expanded.elements;
    }
    const layers = [];
    const moves = [];
    // What's in it, each with what it's in, for a page's `<svg>`, whose elements code can look for.
    const found = told === undefined ? undefined : [];
    const font = { fontSize: props.fontSize, fontFamily: props.fontFamily, fontWeight: props.fontWeight, fontStyle: props.fontStyle };
    addLayers(children, { fill, fillRule, stroke, strokeWidth, strokeLinecap, strokeLinejoin, strokeMiterlimit, strokeDasharray, strokeDashoffset, parts, moves, found, foundAt: -1, font }, layers);
    told?.(found);
    // Stretched to fill its box either way, rather than kept as it's shaped, as SVG's `preserveAspectRatio="none"` says.
    if (preserveAspectRatio === 'none') {
        props.stretch = true;
    }
    // What CSS says moves in a part moves the whole shape's layers, numbers and colors and all.
    let transitions = props.transitions;
    if (moves.length > 0) {
        const longest = moves.reduce((most, one) => (one.duration + one.delay > most.duration + most.delay ? one : most));
        const line = `shape_layers s ${longest.duration} ${longest.delay} ${longest.easing}`;
        transitions = transitions === undefined ? line : transitions + '\n' + line;
    }
    return element('image', { ...props, viewBox, shapeLayers: layers.join('\n'), transitions });
}

/// A switch that's on or off. `label` shows beside it, and it's what it's called either way.
function Switch({ id, checked, label, disabled, onChange, ...rest }) {
    const along = useAnimated(checked ? 1 : 0, 120);
    const [WIDTH, HEIGHT, THUMB] = [34, 18, 14];
    const track = element('group', {
        width: WIDTH,
        height: HEIGHT,
        radius: HEIGHT / 2,
        direction: 'row',
        align: 'center',
        paddingLeft: 2 + along * (WIDTH - THUMB - 4),
        background: checked ? '#1f6feb' : widgets.edge,
    }, element('group', {
        width: THUMB,
        height: THUMB,
        radius: THUMB / 2,
        background: '#ffffff',
    }));
    return element('checkbox', {
        key: id,
        name: label,
        checked,
        disabled,
        direction: 'row',
        align: 'center',
        gap: 8,
        ...rest,
        onToggle: onChange === undefined ? undefined : () => onChange(!checked),
    }, track, label === undefined ? null : element('text', {}, label));
}

/// One of a few choices, of which `value` is the number of the one that's chosen. The arrow keys move between them.
function RadioGroup({ id = 'radio', options, value, disabled, name, direction = 'column', gap = 6, onChange }) {
    const choose = (index) => {
        if (index !== value && onChange !== undefined) {
            onChange(index, options[index]);
        }
    };
    const count = options.length;
    return element('group', {
        key: id,
        name,
        direction,
        gap,
        onKeyPress: (key) => {
            if (disabled) {
                return false;
            }
            if (key === 'down' || key === 'right') {
                const next = (value + 1) % count;
                choose(next);
                focus(`${id}-${next}`);
                return true;
            }
            if (key === 'up' || key === 'left') {
                const next = (value + count - 1) % count;
                choose(next);
                focus(`${id}-${next}`);
                return true;
            }
            return false;
        },
    }, options.map((option, index) => {
        const chosen = index === value;
        const ring = element('group', {
            width: 16,
            height: 16,
            radius: 8,
            align: 'center',
            justify: 'center',
            background: widgets.field,
            borderColor: chosen ? '#1f6feb' : widgets.edge,
            borderWidth: chosen ? 5 : 1,
        });
        return element('checkbox', {
            key: `${id}-${index}`,
            name: option,
            checked: chosen,
            disabled,
            direction: 'row',
            align: 'center',
            gap: 8,
            onToggle: () => choose(index),
        }, ring, element('text', {}, option));
    }));
}

/// How far something's got, from 0 to `max`. Nothing's said for how far is what `Spinner` is for.
function ProgressBar({ id, value, max = 1, width, name }) {
    const share = max > 0 ? Math.max(0, Math.min(1, value / max)) : 0;
    const shown = useAnimated(share, 120);
    return element('progress', {
        key: id,
        name,
        value: String(Math.round(share * 100)) + '%',
        width,
        height: 6,
        radius: 3,
        direction: 'row',
        background: widgets.line,
    }, element('group', { width: `${shown * 100}%`, height: 6, radius: 3, background: '#1f6feb' }));
}

/// Something's going on, for however long it takes. It stands still if the system's asked for things not to move.
function Spinner({ size = 16, color = '#1f6feb', name = 'Busy' }) {
    const [turn, setTurn] = useState(0);
    useEffect(() => {
        if (!host.animationsOn()) {
            return undefined;
        }
        const timer = setInterval(() => setTurn((now) => (now + 1) % 12), 80);
        return () => clearInterval(timer);
    }, []);
    // Twelve pictures, of which only the one that's showing is drawn, and they're kept.
    const angle = (turn / 12) * 2 * Math.PI;
    const at = (around) => `${12 + 9 * Math.cos(around)} ${12 + 9 * Math.sin(around)}`;
    return element(Svg, {
        name,
        viewBox: '0 0 24 24',
        width: size,
        height: size,
        fill: 'none',
        strokeWidth: 3,
        strokeLinecap: 'round',
        stroke: color,
    }, element(Path, { d: `M${at(angle)}A9 9 0 1 1 ${at(angle + 4.7)}` }));
}

/// A row of tabs, of which `value` is the number of the one that's chosen. What goes with each is up to what's using this.
function Tabs({ id = 'tabs', options, value, onChange, gap = 2 }) {
    const count = options.length;
    const choose = (index) => {
        if (index !== value && onChange !== undefined) {
            onChange(index, options[index]);
        }
    };
    return element('group', {
        key: id,
        direction: 'row',
        gap,
        onKeyPress: (key) => {
            if (key === 'right' || key === 'left') {
                const next = (value + (key === 'right' ? 1 : count - 1)) % count;
                choose(next);
                focus(`${id}-${next}`);
                return true;
            }
            return false;
        },
    }, options.map((option, index) => {
        const chosen = index === value;
        return element('button', {
            key: `${id}-${index}`,
            name: option,
            selected: chosen,
            paddingX: 12,
            paddingY: 7,
            hover: widgets.soft,
            radius: 4,
            onClick: () => choose(index),
        }, element('column', { gap: 4, align: 'center' },
            element('text', { fontWeight: chosen ? 'bold' : undefined }, option),
            element('group', { width: '100%', height: 2, background: chosen ? '#1f6feb' : 'transparent' })));
    }));
}

/// Another window, for as long as this is here, which is all that's needed to open and close one:
/// `{open && <Window title="Settings" onClose={() => setOpen(false)}>...</Window>}`. What's inside is part of this app like
/// anything else, with the same state and context, and it can go on changing along with what's around it.
function Window({ title, size, minSize, modal, owned, frameless, alwaysOnTop, fullscreen, layer, transparent, skipTaskbar, allDesktops, appBar, onClose, rememberPlacement, children, ...props }) {
    const owner = current;
    const [entry] = useState(() => ({ id: null, owner, shows: null, redraw: null, closed: false }));
    // Clicking the close button is a question, which is for `onClose` to answer. With nothing to ask, the answer's yes.
    const asked = () => {
        if (entry.onClose !== undefined) {
            entry.onClose();
        } else {
            entry.closed = true;
            host.closeWindow(entry.id);
        }
    };
    entry.onClose = onClose;
    entry.shows = element('window', { ...props, onClose: asked }, children);
    if (entry.redraw !== null) {
        entry.redraw();
    }
    useLastingEffect(() => {
        entry.id = host.openWindow(entry, title ?? '', size?.[0], size?.[1], minSize?.[0], minSize?.[1], modal === true, owned !== false, frameless === true, alwaysOnTop === true, fullscreen === true, windowLayer(layer, alwaysOnTop), transparent === true, skipTaskbar === true, allDesktops === true, appBarOf(appBar));
        return () => {
            if (!entry.closed) {
                entry.closed = true;
                host.closeWindow(entry.id);
            }
        };
    }, []);
    useEffect(() => {
        if (entry.id !== null && !entry.closed) {
            host.setWindowTitle(entry.id, title ?? '');
            windowKnown(entry.id).title = title ?? '';
        }
    }, [title]);
    usePlacementKept(rememberPlacement, () => entry.id);
    // These are how it opened, the first time, and each is only said again when it changes, so that changing one leaves
    // what's been done to the others since, as by `WindowControls`, as it is.
    const changeLater = (deps, change) => {
        const seen = useRef(false);
        useEffect(() => {
            if (seen.current && entry.id !== null && !entry.closed) {
                change();
            }
            seen.current = true;
        }, deps);
    };
    changeLater([frameless], () => host.changeWindow(entry.id, 'frameless', frameless === true));
    changeLater([fullscreen], () => host.changeWindow(entry.id, 'fullscreen', fullscreen === true));
    const wantedLayer = windowLayer(layer, alwaysOnTop);
    changeLater([wantedLayer], () => host.changeWindow(entry.id, 'layer', wantedLayer));
    changeLater([transparent === true], () => host.changeWindow(entry.id, 'transparent', transparent === true));
    changeLater([skipTaskbar === true], () => host.changeWindow(entry.id, 'skipTaskbar', skipTaskbar === true));
    changeLater([allDesktops === true], () => host.changeWindow(entry.id, 'allDesktops', allDesktops === true));
    const bar = appBarOf(appBar);
    changeLater([bar?.edge, bar?.size, bar?.monitor], () => host.changeWindow(entry.id, 'appBar', bar));
    return null;
}

const windowLayers = ['normal', 'top', 'bottom', 'desktop'];

/// The layer a window's in, from `layer`, or `alwaysOnTop` for `top` when that doesn't say.
function windowLayer(layer, alwaysOnTop) {
    if (layer !== undefined && layer !== null) {
        if (!windowLayers.includes(layer)) {
            throw new TypeError(`A window's layer is 'normal', 'top', 'bottom' or 'desktop', not ${JSON.stringify(layer)}`);
        }
        return layer;
    }
    return alwaysOnTop === true ? 'top' : 'normal';
}

/// An app bar as it's handed on, `{ edge, size, monitor }`, or `null` for none.
function appBarOf(bar) {
    if (bar === undefined || bar === null || bar === false) {
        return null;
    }
    if (!['top', 'bottom', 'left', 'right'].includes(bar.edge)) {
        throw new TypeError(`An app bar's edge is 'top', 'bottom', 'left' or 'right', not ${JSON.stringify(bar.edge)}`);
    }
    const size = Number(bar.size);
    if (!(size > 0)) {
        throw new TypeError(`An app bar's size is a number of pixels more than 0, not ${JSON.stringify(bar.size)}`);
    }
    const out = { edge: bar.edge, size };
    if (bar.monitor !== undefined && bar.monitor !== null) {
        out.monitor = Number(bar.monitor);
    }
    return out;
}

/// What can be done to a window that's open.
class WindowControls {
    #id;

    constructor(id) {
        this.#id = id;
    }

    get id() {
        return this.#id;
    }

    setFrameless(on = true) {
        host.changeWindow(this.#id, 'frameless', on === true);
    }

    setAlwaysOnTop(on = true) {
        host.changeWindow(this.#id, 'onTop', on === true);
    }

    /// Which of the system's layers it's in: `normal`; `top`, above other programs' windows, as `setAlwaysOnTop` has it;
    /// `bottom`, beneath every other window, staying there when it's clicked; or `desktop`, on the desktop behind its icons,
    /// where Show Desktop leaves it, and where the pointer doesn't reach it.
    setLayer(layer = 'normal') {
        host.changeWindow(this.#id, 'layer', windowLayer(layer, false));
    }

    /// Whether only what's drawn in it shows, at the opacity it's drawn with, with what's behind it everywhere else. Leave the
    /// root's `background` out, or make it `transparent`. Where nothing's drawn, the pointer goes through to what's behind.
    setTransparent(on = true) {
        host.changeWindow(this.#id, 'transparent', on === true);
    }

    /// Keeps a strip along an edge of a screen for itself, as the taskbar does, and moves there: `{ edge, size, monitor }`,
    /// with `size` in logical pixels and `monitor` counting from 0 for the main screen. `null` gives it back.
    setAppBar(bar) {
        host.changeWindow(this.#id, 'appBar', appBarOf(bar));
    }

    /// Whether it shows on every virtual desktop. That makes it a tool window, which is left off the taskbar and Alt+Tab too.
    setAllDesktops(on = true) {
        host.changeWindow(this.#id, 'allDesktops', on === true);
    }

    setFullscreen(on = true) {
        host.changeWindow(this.#id, 'fullscreen', on === true);
        this.#known({ fullscreen: on === true });
    }

    minimize() {
        host.changeWindow(this.#id, 'minimize');
        this.#known({ minimized: true, maximized: false });
    }

    maximize() {
        host.changeWindow(this.#id, 'maximize');
        this.#known({ minimized: false, maximized: true });
    }

    restore() {
        host.changeWindow(this.#id, 'restore');
        this.#known({ minimized: false, maximized: false });
    }

    hide() {
        host.changeWindow(this.#id, 'hide');
        this.#known({ visible: false });
    }

    show() {
        host.changeWindow(this.#id, 'show');
        this.#known({ visible: true });
    }

    focus() {
        host.changeWindow(this.#id, 'focus');
        for (const [id, known] of windowsKnown) {
            known.focused = id === this.#id;
        }
        this.#known({ focused: true });
    }

    /// What's been asked of it, for when the system can't say, and for what's listening.
    #known(changes) {
        Object.assign(windowKnown(this.#id), changes);
        windowStateChanged(this.#id);
    }

    center() {
        host.changeWindow(this.#id, 'center');
    }

    moveTo(x, y) {
        host.changeWindow(this.#id, 'move', x, y);
        this.#known({ x: Number(x), y: Number(y) });
    }

    resize(width, height) {
        host.changeWindow(this.#id, 'resize', width, height);
        this.#known({ width: Number(width), height: Number(height) });
    }

    /// The text in its title bar, and on its button in the taskbar.
    setTitle(title) {
        const text = String(title);
        if (this.#id === 0) {
            host.setTitle(text);
        } else {
            host.setWindowTitle(this.#id, text);
        }
        windowKnown(this.#id).title = text;
        windowStateChanged(this.#id);
    }

    /// How much of it shows, from 0, nothing, to 1, all of it.
    setOpacity(value) {
        host.changeWindow(this.#id, 'opacity', Math.min(1, Math.max(0, Number(value))));
    }

    /// Whether its edges can be dragged to make it bigger or smaller, and it can be maximized.
    setResizable(on = true) {
        host.changeWindow(this.#id, 'resizable', on === true);
    }

    /// How small and how big it can be made inside, in logical pixels: `{ minWidth, minHeight, maxWidth, maxHeight }`, each if
    /// that's limited.
    setSizeLimits({ minWidth, minHeight, maxWidth, maxHeight } = {}) {
        host.changeWindow(this.#id, 'sizeLimits', minWidth, minHeight, maxWidth, maxHeight);
    }

    /// Whether it's left off the taskbar and what Alt+Tab shows.
    setSkipTaskbar(on = true) {
        host.changeWindow(this.#id, 'skipTaskbar', on === true);
    }

    /// Its own icon, in its title bar and on the taskbar: an `.ico` file or a picture, or one's bytes.
    setIcon(icon) {
        host.changeWindow(this.#id, 'icon', require('rsui/desktop/core').pictureBytes(icon));
    }

    /// Whether the pointer goes straight through it, to what's behind, as for something laid over the screen.
    setClickThrough(on = true) {
        host.changeWindow(this.#id, 'clickThrough', on === true);
    }

    /// Whether it's left out of screenshots and recordings, which show black in its place.
    setContentProtection(on = true) {
        host.changeWindow(this.#id, 'contentProtection', on === true);
    }

    /// Shown without taking the keyboard from what has it, as a notification of an app's own would be.
    showWithoutActivation() {
        host.changeWindow(this.#id, 'showInactive');
    }

    /// Starts making it bigger or smaller from an edge, as if that were dragged, for a window with no frame: `left`, `right`,
    /// `top`, `bottom`, `top-left`, `top-right`, `bottom-left` or `bottom-right`. Only while a button's held.
    startResize(edge = 'bottom-right') {
        host.changeWindow(this.#id, 'startResize', String(edge));
    }

    /// How it is now, in logical pixels: `{ title, x, y, width, height, visible, minimized, maximized, fullscreen, focused }`.
    getState() {
        return windowStateOf(this.#id);
    }

    /// Runs with how it is each time that changes. What it gives back stops it.
    onStateChange(run) {
        return listenToWindowState(this.#id, run);
    }

    /// A picture of what's in it, as a PNG file's bytes. It's drawn for this once what's running now is done.
    capture() {
        return new Promise((resolve, reject) => {
            const { action, forget } = require('rsui/desktop/core');
            const id = action((said) => {
                forget([id]);
                const text = String(said);
                if (text.startsWith('png ')) {
                    resolve(Uint8Array.from(text.slice(4).match(/../g) ?? [], (pair) => parseInt(pair, 16)));
                } else {
                    reject(new Error(text.replace(/^error /, '')));
                }
            });
            try {
                host.windowCapture(this.#id, id);
            } catch (error) {
                forget([id]);
                reject(error);
            }
        });
    }

    /// Puts a menu bar on it, or takes it off with `null`. Each item has a `label`, and an `onClick`, `items` for what opens
    /// from it, or `separator: true`; `shortcut` is keys that do the same, like `ctrl+o`, and `disabled` and `checked` are
    /// what they say.
    setMenu(menu) {
        if (this.#id !== 0) {
            throw new Error('Only the main window can have a menu bar yet');
        }
        const { forget, menuSpec } = require('rsui/desktop/core');
        forget(menuActions);
        menuActions = [];
        host.setMenuBar(menuSpec(menu ?? [], menuActions));
    }

    /// Closes it. For the main window, that ends the program, whatever its `onClose` would have said.
    close() {
        host.closeWindow(this.#id);
    }
}

let menuActions = [];

/// What's known of each window by what it's been asked to do and told, for when the system has nothing to say about it, as when
/// an app's being checked rather than shown.
const windowsKnown = new Map();
function windowKnown(id) {
    let known = windowsKnown.get(id);
    if (known === undefined) {
        const size = globalThis.__rsuiPageSize ?? [800, 600];
        known = { title: id === 0 ? host.mainTitle() : '', x: 0, y: 0, width: size[0], height: size[1], visible: true, minimized: false, maximized: false, fullscreen: false, focused: id === 0 };
        windowsKnown.set(id, known);
    }
    return known;
}

function windowStateOf(id) {
    const real = host.windowState(id);
    if (real !== null && real !== undefined) {
        return real;
    }
    const known = windowKnown(id);
    if (id === 0 && globalThis.__rsuiPageSize !== undefined) {
        [known.width, known.height] = globalThis.__rsuiPageSize;
    }
    return { ...known };
}

const stateListeners = new Map();
const stateSaid = new Map();
let stateHeard = false;

/// Tells what's listening to a window's state if it's different from what they were last told.
function windowStateChanged(id) {
    const listeners = stateListeners.get(id);
    if (listeners === undefined || listeners.size === 0) {
        return;
    }
    const now = windowStateOf(id);
    const said = JSON.stringify(now);
    if (stateSaid.get(id) === said) {
        return;
    }
    stateSaid.set(id, said);
    for (const listener of [...listeners]) {
        listener(now);
    }
}

function listenToWindowState(id, run) {
    if (!stateHeard) {
        stateHeard = true;
        const { action } = require('rsui/desktop/core');
        host.windowOnState(action((which) => windowStateChanged(Number(which))));
    }
    let listeners = stateListeners.get(id);
    if (listeners === undefined) {
        listeners = new Set();
        stateListeners.set(id, listeners);
        stateSaid.set(id, JSON.stringify(windowStateOf(id)));
    }
    listeners.add(run);
    return () => listeners.delete(run);
}

/// How this window is, for as long as this component's there, which it renders again with when it changes.
function useWindowState() {
    const win = useCurrentWindow();
    const [state, setState] = useState(() => win.getState());
    useEffect(() => win.onStateChange(setState), [win]);
    return state;
}

const mainWindow = new WindowControls(0);

/// Puts a window back where it was the last time, as `rememberPlacement="settings"` says, and keeps where it is from then on, in
/// the app's own store. What isn't on any screen now is moved onto the main one.
function usePlacementKept(key, idOf) {
    useLastingEffect(() => {
        if (typeof key !== 'string' || key === '') {
            return undefined;
        }
        const id = idOf();
        if (id === null || id === undefined) {
            return undefined;
        }
        const { storage } = require('rsui/desktop/storage');
        const kept = storage.get('__placement:' + key);
        const controls = new WindowControls(id);
        if (kept !== undefined && kept !== null && typeof kept === 'object') {
            const screens = host.displays();
            const onScreen = screens.some((one) => {
                const scale = one.scale || 1;
                const [left, top] = [one.x / scale, one.y / scale];
                return kept.x + 40 > left && kept.x < left + one.width / scale - 40 && kept.y >= top - 8 && kept.y < top + one.height / scale - 40;
            });
            if (kept.width > 0 && kept.height > 0) {
                controls.resize(kept.width, kept.height);
            }
            if (onScreen) {
                controls.moveTo(kept.x, kept.y);
            } else {
                controls.center();
            }
            if (kept.maximized === true) {
                controls.maximize();
            }
        }
        let soon = null;
        const stop = controls.onStateChange((state) => {
            if (state.minimized || state.fullscreen) {
                return;
            }
            clearTimeout(soon);
            // Where it ends up, rather than each step of being dragged there.
            soon = setTimeout(() => storage.set('__placement:' + key, { x: state.x, y: state.y, width: state.width, height: state.height, maximized: state.maximized }), 250);
        });
        return () => {
            clearTimeout(soon);
            stop();
        };
    }, []);
}

/// The window that this component is in.
function useCurrentWindow() {
    let at = current;
    while (at !== null && at.more?.windowEntry === undefined) {
        at = at.parent;
    }
    const entry = at === null ? null : at.more.windowEntry;
    return useMemo(() => (entry === null ? mainWindow : new WindowControls(entry.id)), [entry]);
}

/// What's at the top of a `Window`. It comes under what made it, so that context reaches it, though it's a tree of its own.
function WindowRoot(entry) {
    current.parent = entry.owner;
    moreOf(current).windowEntry = entry;
    const [, setDrawn] = useState(0);
    entry.redraw = () => setDrawn((drawn) => drawn + 1);
    useLastingEffect(() => {
        // It goes with the rest, if the app's said it goes along with the system.
        if (colorSchemeWatchers.size > 0) {
            host.setDarkTitleBar(colorScheme === 'dark');
        }
        return () => {
            entry.redraw = null;
        };
    }, []);
    return entry.shows;
}

const POPUP_FADE = 120;
const POPUP_FADE_OUT = 90;

/// Something that takes over the window while `open` is true, with the rest dimmed behind it. It has the keyboard to
/// itself, which goes back to where it was afterwards. Clicking away or pressing escape calls `onClose`.
function Dialog({ open, onClose, name, children, id = 'dialog' }) {
    const shown = useAnimated(open ? 1 : 0, open ? POPUP_FADE : POPUP_FADE_OUT);
    if (!open && shown <= 0) {
        return null;
    }
    return element('dialog', {
        key: id,
        name,
        overlay: 'fill',
        align: 'center',
        justify: 'center',
        background: widgets.dim,
        color: widgets.ink,
        opacity: shown < 0.999 ? shown : undefined,
        inert: open ? undefined : true,
        onDismiss: open ? onClose : undefined,
    }, element('group', {
        key: 'panel',
        padding: 20,
        gap: 12,
        maxWidth: 560,
        background: widgets.panel,
        radius: 10,
        shadowBlur: 24,
        shadowY: 8,
        shadowColor: '#00000033',
    }, children));
}

/// A button that opens a menu of `items`, each with a `label` and an `onSelect`. The arrow keys go through them.
function MenuButton({ id = 'menu', label, items, disabled }) {
    const [open, setOpen] = useState(false);
    const shown = useAnimated(open ? 1 : 0, open ? POPUP_FADE : POPUP_FADE_OUT);
    const button = element('button', {
        key: id + '-button',
        disabled,
        paddingX: 12,
        paddingY: 6,
        background: widgets.button,
        color: widgets.ink,
        hover: widgets.buttonHover,
        pressed: widgets.buttonPressed,
        radius: 6,
        onClick: () => setOpen((was) => !was),
    }, label);
    let menu = null;
    if (open || shown > 0) {
        menu = element('menu', {
            key: id + '-menu',
            overlay: 'below',
            padding: 4,
            minWidth: 120,
            background: widgets.panel,
            color: widgets.ink,
            borderColor: widgets.line,
            borderWidth: 1,
            radius: 8,
            shadowBlur: 14,
            shadowY: 4,
            shadowColor: '#00000026',
            opacity: shown < 0.999 ? shown : undefined,
            inert: open ? undefined : true,
            onDismiss: open ? () => setOpen(false) : undefined,
        }, items.map((item, index) => element('menuitem', {
            key: id + '-item-' + index,
            paddingX: 10,
            paddingY: 6,
            hover: widgets.soft,
            radius: 4,
            onClick: () => {
                setOpen(false);
                if (item.onSelect) {
                    item.onSelect();
                }
            },
        }, item.label)));
    }
    return element('group', { key: id, align: 'start' }, button, menu);
}

/// A button showing which of `options` is chosen, which opens a menu of them.
function Select({ id = 'select', options, value, onChange, disabled }) {
    const items = options.map((option, index) => ({
        label: option,
        onSelect: () => onChange(index, option),
    }));
    return element(MenuButton, { id, label: options[value] ?? '', items, disabled });
}

/// One line of editable text, with a border. `value` and `onChange` are what any input has. Give it `rows` for several.
/// A value between `min` and `max`, set by dragging, by clicking along it, or with the arrow keys.
function Slider({ id, value, min = 0, max = 100, step = 1, width: said, naturalWidth, onChange, name }) {
    const width = said ?? naturalWidth ?? 200;
    const clamp = (next) => {
        const stepped = Math.round((next - min) / step) * step + min;
        return Math.max(min, Math.min(max, stepped));
    };
    const fraction = max > min ? (value - min) / (max - min) : 0;
    const THUMB = 16;
    const room = width - THUMB;
    const set = (next) => {
        const clamped = clamp(next);
        if (clamped !== value && onChange !== undefined) {
            onChange(clamped);
        }
    };
    // The part that's been filled in, the thumb and what's left, side by side, so that nothing needs placing by hand.
    return element('button', {
        key: id,
        name: name ?? String(value),
        direction: 'row',
        align: 'center',
        width,
        height: THUMB,
        radius: THUMB / 2,
        value: String(value),
        onClick: () => {},
        onDrag: (phase, x) => {
            if (phase === 'start' || phase === 'move') {
                set(min + ((x - THUMB / 2) / room) * (max - min));
            }
        },
        onKeyPress: (key) => {
            const moves = { left: value - step, right: value + step, down: value - step, up: value + step, home: min, end: max };
            if (key in moves) {
                set(moves[key]);
                return true;
            }
            return false;
        },
    },
    element('group', { width: fraction * room, height: 4, background: '#1f6feb' }),
    element('group', { width: THUMB, height: THUMB, radius: THUMB / 2, background: '#1f6feb' }),
    element('group', { flex: 1, height: 4, background: widgets.line }));
}

// What the clipboard had, when it was asked for: `rsui/desktop`'s `clipboard` says what to tell.
let pastedTo = null;

function pasted(text) {
    pastedTo?.(text);
}

// What the ready-made widgets are drawn with. They're for a light window, unless the app has asked which it's in.
const LIGHT_WIDGETS = { field: '#ffffff', line: '#d0d7de', soft: '#eaeef2', button: '#d8dee4', buttonHover: '#c9d1d9', buttonPressed: '#afb8c1', edge: '#8c959f', panel: '#ffffff', ink: '#1f2328', dim: '#00000066' };
const DARK_WIDGETS = { field: '#0d1117', line: '#30363d', soft: '#21262d', button: '#21262d', buttonHover: '#30363d', buttonPressed: '#484f58', edge: '#6e7681', panel: '#161b22', ink: '#e6edf3', dim: '#00000099' };
let widgets = LIGHT_WIDGETS;

let colorScheme = null;
const colorSchemeWatchers = new Set();

function currentColorScheme() {
    if (colorScheme === null) {
        colorScheme = host.isDark() ? 'dark' : 'light';
    }
    return colorScheme;
}

/// Whether the system's set to `'dark'` or `'light'`, which changes along with it. Asking means the app
/// goes along with it, so the window's title bar does too.
function useColorScheme() {
    const [shown, setShown] = useState(currentColorScheme);
    widgets = shown === 'dark' ? DARK_WIDGETS : LIGHT_WIDGETS;
    useEffect(() => {
        const scheme = currentColorScheme();
        setShown(scheme);
        host.setDarkTitleBar(scheme === 'dark');
        colorSchemeWatchers.add(setShown);
        return () => colorSchemeWatchers.delete(setShown);
    }, []);
    return shown;
}

/// How big the window is, in logical pixels, for what a page asks about it. `__rsuiPageResized` is told when it changes.
function windowResized(width, height) {
    const before = globalThis.__rsuiPageSize;
    globalThis.__rsuiPageSize = [width, height];
    if (windowsKnown.has(0)) {
        Object.assign(windowKnown(0), { width, height });
    }
    windowStateChanged(0);
    if (before === undefined || before[0] !== width || before[1] !== height) {
        globalThis.__rsuiPageResized?.();
    }
}

/// What's told when the system goes from light to dark or back, as `matchMedia` is.
const colorSchemeHeard = new Set();

function colorSchemeChanged(dark) {
    colorScheme = dark ? 'dark' : 'light';
    for (const heard of colorSchemeHeard) {
        heard(colorScheme);
    }
    if (colorSchemeWatchers.size > 0) {
        host.setDarkTitleBar(dark);
        widgets = dark ? DARK_WIDGETS : LIGHT_WIDGETS;
        // A widget with the same props wouldn't render again otherwise, and would stay as it was.
        everything += 1;
        host.dirtyAll('The system went from light to dark, or back.');
    }
    for (const watcher of colorSchemeWatchers) {
        watcher(colorScheme);
    }
}

/// Gives the keyboard to what has this key. `visible`, if it's said, is whether that shows (a focus ring, `:focus-visible`), as a page's
/// `focus({ focusVisible })` says; otherwise that follows how the keyboard was last moved.
function focus(key, visible, preventScroll = false) {
    host.focus(key, typeof visible === 'boolean' ? visible : undefined, preventScroll);
}

function TextInput({ autoFocus = false, ...props }) {
    const key = props.key;
    useEffect(() => {
        if (autoFocus && key !== undefined) {
            focus(key);
        }
    }, []);
    return element('input', {
        paddingX: 10,
        paddingY: 6,
        background: widgets.field,
        color: widgets.ink,
        borderColor: widgets.line,
        borderWidth: 1,
        radius: 6,
        ...props,
    });
}

/// What each of a component's hooks is, where it's called from, and what it holds, for a debugger.
function describeHooks(instance) {
    const described = instance.hooks.map((hook) => {
        const kind = kindName(hook.kind);
        let value;
        switch (kind) {
            case 'state': case 'reducer': case 'memo': value = hook.value; break;
            case 'ref': value = hook.current; break;
            case 'effect': value = hook.deps; break;
            case 'animated': value = hook.to; break;
            default: value = hook.value;
        }
        return [kind, hook.site, value];
    });
    // What it reads from a context isn't a hook, but it's shown with them, where it's read.
    for (const provider of instance.watching ?? []) {
        described.push(['context', instance.more?.contextSites?.get(provider), provider.value]);
    }
    return described;
}

/// Puts a component's state back to what it was, for a debugger going back through what's changed, without that counting as
/// a change of its own. Says whether it could.
function restoreState(instance, index, value) {
    const hook = instance.hooks[index];
    if ((instance.flags & DEAD) !== 0 || hook === undefined || (hook.kind !== 'state' && hook.kind !== 'reducer')) {
        return false;
    }
    if (!Object.is(hook.value, value)) {
        hook.value = value;
        markDirty(instance);
    }
    return true;
}


// What a page's element written in JSX comes to, as `<div className="x">`, which is `rsui/web`'s. That's loaded the first time one's
// used, so an app that doesn't write any pays nothing for it. Rust asks this only about tags a page has: see `bridge.rs`.
let pageElement = null;
function webDescription(description) {
    pageElement ??= require('rsui/web').__pageElement;
    const type = pageElement(description.type);
    if (type === undefined) {
        return undefined;
    }
    const made = __jsx(type, description.props ?? NO_PROPS, description.kids);
    made.key = description.key;
    // What the app's own JSX wrote is still the app's, so that it can be the same as what was written before (see `jsxMemo`).
    if (description.memo === true && type.__rsuiEquivalent === true) {
        made.memo = pageMark();
    }
    return made;
}

// What a click handler gives back for the click to go on to what's around it that takes clicks, which is what `rsui/web` does.
const PASS_CLICK = Object.freeze({});

// What a click handler is given, with what came with it, if anything did.
function newClickEvent(within, info) {
    const event = info ?? Object.create(clickEvent);
    event.type = 'click';
    event.__within = within ?? 0;
    event.nativeEvent = event;
    return event;
}

// What's listening on a page's `document` and `window`, from 'rsui/web', if that's been loaded.
let documentHeard = null;

// What Rust asks of 'rsui/web' about a page's plain element drawn in place, which only exists once that's loaded: see
// `placeElement` there.
function placeElement(description, owner, parent, place) {
    return module.exports.__placeElement(description, owner, parent, place);
}

function placedWalked(owner) {
    return module.exports.__placedWalked?.(owner) ?? false;
}

globalThis.__rsuiInternals = { rejected, placeElement, placedWalked, setTracing(on) { tracing = on; }, describeHooks, restoreState, desktopEvent: (id, value) => desktopEvents?.(id, value), webDescription, newClickEvent, newEvent, rsuiEvents: (given) => require('rsui/events').rsuiEvents(given), documentEvent: (kind, event, prevented) => documentHeard?.(kind, event, prevented), passClick: PASS_CLICK, notKept: NOT_KEPT, pasted, colorSchemeChanged, windowResized, WindowRoot, newInstance, render, flushEffects, commitEffects, unmount, setReloading(on) { reloading = on; } };

/// A promise that was rejected with nothing to catch it, once everything that was waiting has run, as Rust says. The window hears
/// `unhandledrejection` about it, as a browser's does, and it's logged unless something there calls `preventDefault()`. The app
/// goes on, as a page does: it's only an error that shows over the app when it's thrown while rendering or in a handler.
function rejected(reason, promise) {
    const event = {
        type: 'unhandledrejection',
        reason,
        promise,
        bubbles: false,
        cancelable: true,
        isTrusted: true,
        defaultPrevented: false,
        eventPhase: 2,
        timeStamp: performance.now(),
        target: globalThis,
        currentTarget: globalThis,
        srcElement: globalThis,
        preventDefault() {
            event.defaultPrevented = true;
        },
        stopPropagation() {
            event.__stopped = true;
        },
        stopImmediatePropagation() {
            event.__stopped = true;
            event.__immediate = true;
        },
    };
    try {
        if (typeof globalThis.dispatchEvent === 'function') {
            globalThis.dispatchEvent(event);
        } else if (typeof globalThis.onunhandledrejection === 'function') {
            globalThis.onunhandledrejection(event);
        }
    } catch (error) {
        console.error(error);
    }
    if (!event.defaultPrevented) {
        console.error('Uncaught (in promise)', reason);
    }
}

// What hears about things happening on the desktop, from 'rsui/desktop', if that's been loaded.
let desktopEvents = null;

// What a handler is given about something that happened, as the web has it. `stopPropagation()` keeps it from what's around
// what it happened to, and `preventDefault()` from doing what it would by itself, as a key typing or a press moving the keyboard.
const eventBase = {
    type: '',
    bubbles: true,
    cancelable: true,
    isTrusted: true,
    defaultPrevented: false,
    target: null,
    currentTarget: null,
    relatedTarget: null,
    pointerId: 1,
    pointerType: 'mouse',
    isPrimary: true,
    width: 1,
    height: 1,
    pressure: 0,
    deltaZ: 0,
    deltaMode: 0,
    charCode: 0,
    preventDefault() {
        if (!this.defaultPrevented) {
            this.defaultPrevented = true;
            host.preventEvent();
        }
    },
    isDefaultPrevented() {
        return this.defaultPrevented;
    },
    stopPropagation() {
        this.__stopped = true;
        host.stopEvent();
    },
    stopImmediatePropagation() {
        this.__immediate = true;
        this.stopPropagation();
    },
    // On rsui's own elements, a click goes on to the nearest `onClick` around it, which it doesn't otherwise.
    passOn() {
        host.passEvent();
    },
    isPropagationStopped() {
        return this.__stopped === true;
    },
    persist() {},
    getModifierState(name) {
        return { Control: this.ctrlKey, Shift: this.shiftKey, Alt: this.altKey, Meta: this.metaKey }[name] === true;
    },
    // What it goes through, nearest first: what it happened to, what that's in, and then the page, as `rsui/web` says it.
    composedPath() {
        if (Array.isArray(this.__path)) {
            return [...this.__path];
        }
        return eventPath === null ? [] : eventPath(this);
    },
};

// What a key is as the web's old `keyCode` has it, which a lot of code still looks at.
const KEY_CODES = { Enter: 13, Escape: 27, Tab: 9, Backspace: 8, Delete: 46, ' ': 32, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, Home: 36, End: 35, PageUp: 33, PageDown: 34, Insert: 45, Shift: 16, Control: 17, Alt: 18, Meta: 91, CapsLock: 20, ContextMenu: 93 };
function keyCodeOf(key, code) {
    if (KEY_CODES[key] !== undefined) {
        return KEY_CODES[key];
    }
    if (/^F[0-9]{1,2}$/.test(key)) {
        return 111 + Number(key.slice(1));
    }
    if (typeof code === 'string' && (code.startsWith('Key') || code.startsWith('Digit'))) {
        return code.charCodeAt(code.length - 1);
    }
    return typeof key === 'string' && key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0;
}

// What 'rsui/web' says about what an event happened to, given its event, if it's been loaded.
let eventTargets = null;
// What 'rsui/web' says an event goes through, for `composedPath()`, if it's been loaded.
let eventPath = null;

// The event `newEvent` last made, by Rust's number for it, and what was along the way to it, for the next handler it goes to.
const lastEvent = { serial: 0, event: null, chain: undefined, depth: 0 };

// Fills in an event from what Rust says about it: see `event_value` in `bridge.rs`. One Rust says it's said before (by its
// number) is the same object, as a page's event is the same one for every listener it goes to, wherever it's heard.
function newEvent(numbers, key, code, data, chain, files) {
    const [x, y, screenX, screenY, movementX, movementY, button, buttons, held, detail, phase, deltaX, deltaY, location, repeat, composing, current, depth, serial] = numbers;
    if (serial !== 0 && serial === lastEvent.serial) {
        const again = lastEvent.event;
        again.eventPhase = phase;
        again.__chain = lastEvent.chain;
        again.__depth = lastEvent.depth;
        again.__current = current;
        if (eventTargets !== null) {
            eventTargets(again, false);
        }
        return again;
    }
    const event = Object.create(eventBase);
    event.clientX = event.pageX = event.x = x;
    event.clientY = event.pageY = event.y = y;
    event.screenX = screenX;
    event.screenY = screenY;
    event.movementX = movementX;
    event.movementY = movementY;
    event.button = button;
    event.buttons = buttons;
    event.pressure = buttons === 0 ? 0 : 0.5;
    event.ctrlKey = (held & 1) !== 0;
    event.shiftKey = (held & 2) !== 0;
    event.altKey = (held & 4) !== 0;
    event.metaKey = (held & 8) !== 0;
    event.detail = detail;
    event.eventPhase = phase;
    event.deltaX = deltaX;
    event.deltaY = deltaY;
    event.timeStamp = performance.now();
    if (key !== undefined) {
        event.key = key;
        event.code = code;
        event.location = location;
        event.repeat = repeat === 1;
        event.isComposing = composing === 1;
        event.keyCode = event.which = keyCodeOf(key, code);
    }
    if (data !== undefined) {
        event.data = data;
    }
    if (files !== undefined) {
        event.__files = files;
    }
    const box = chain?.[depth];
    event.offsetX = event.layerX = box === undefined ? x : x - box[6];
    event.offsetY = event.layerY = box === undefined ? y : y - box[7];
    event.__chain = chain;
    event.__current = current;
    event.__depth = depth;
    event.nativeEvent = event;
    if (eventTargets !== null) {
        eventTargets(event, true);
    } else {
        event.target = event.currentTarget = clickTarget;
    }
    lastEvent.serial = serial;
    lastEvent.event = serial === 0 ? null : event;
    lastEvent.chain = chain;
    lastEvent.depth = depth;
    return event;
}

// What a click handler is given when nothing came with it, as when an agent clicks something by what it's called. It's there
// for what was written for the web, which looks for these. A click only reaches the one thing it's on, unless its handler gives
// back `PASS_CLICK`, as `rsui/web` does for a page's elements. `__within` is how many boxes inside it the press landed in, which
// is 0 when it was on it.
const clickTarget = { focus() {}, blur() {} };
const clickEvent = {
    __proto__: eventBase,
    type: 'click',
    target: clickTarget,
    currentTarget: clickTarget,
    button: 0,
    buttons: 0,
    detail: 1,
    clientX: 0,
    clientY: 0,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    metaKey: false,
};

// Made if they're asked for, so that what doesn't use them doesn't pay for them.
// They're the browser's names for what `rsui/desktop` does, in `rsui/web-routes`.
for (const name of ['Notification', 'Audio', 'alert', 'confirm', 'screen', 'localStorage', 'sessionStorage']) {
    Object.defineProperty(globalThis, name, {
        configurable: true,
        enumerable: false,
        get() {
            const routed = require('rsui/web-routes')[name];
            const value = name === 'localStorage' || name === 'sessionStorage' ? routed() : routed;
            Object.defineProperty(globalThis, name, { value, configurable: true, writable: true, enumerable: false });
            return value;
        },
    });
}
// A library that looks for `window`, `document` or one of the DOM's classes when it's loaded, before the page it's for has made
// anything, finds them, as it would in a browser: the page's world is made then, and not before for what never looks.
const PAGE_WORLD = ['window', 'document', 'Node', 'Element', 'HTMLElement', 'Text', 'Range', 'Selection', 'MutationObserver', 'DOMParser', 'getSelection', 'Event', 'UIEvent', 'InputEvent', 'KeyboardEvent', 'MouseEvent', 'PointerEvent', 'FocusEvent', 'CompositionEvent', 'ClipboardEvent', 'DragEvent', 'WheelEvent'];
for (const name of PAGE_WORLD) {
    const before = Object.getOwnPropertyDescriptor(globalThis, name);
    if (before !== undefined && name !== 'window' && name !== 'document') {
        // The engine's own, as `Event` is, stays as it is.
        continue;
    }
    Object.defineProperty(globalThis, name, {
        configurable: true,
        enumerable: false,
        get() {
            for (const each of PAGE_WORLD) {
                const said = Object.getOwnPropertyDescriptor(globalThis, each);
                if (said?.get?.__pageWorld === true) {
                    delete globalThis[each];
                }
            }
            require('rsui/web');
            return globalThis[name];
        },
        set(value) {
            Object.defineProperty(globalThis, name, { value, configurable: true, writable: true, enumerable: false });
        },
    });
    Object.getOwnPropertyDescriptor(globalThis, name).get.__pageWorld = true;
}
// What a page is told it's in: Edge, as it's Edge's behaviour that a page gets here, with rsui's name after it, from the start,
// as libraries ask when they're loaded.
const PAGE_AGENT = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0 rsui/1.0';
{
    const made = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
    if (typeof made?.get === 'function' && typeof made.set === 'function') {
        Object.defineProperty(globalThis, 'navigator', {
            configurable: true,
            enumerable: made.enumerable,
            get() {
                const navigator = made.get.call(globalThis);
                if (navigator !== null && typeof navigator === 'object') {
                    for (const [name, value] of [['userAgent', PAGE_AGENT], ['appVersion', PAGE_AGENT.slice('Mozilla/'.length)], ['vendor', 'Google Inc.']]) {
                        if (typeof Object.getOwnPropertyDescriptor(navigator, name)?.get !== 'function') {
                            navigator[name] = value;
                        }
                    }
                }
                Object.defineProperty(globalThis, 'navigator', { value: navigator, configurable: true, writable: true, enumerable: made.enumerable });
                return navigator;
            },
            set(value) {
                Object.defineProperty(globalThis, 'navigator', { value, configurable: true, writable: true, enumerable: made.enumerable });
            },
        });
    }
}

module.exports = {
    // For 'react', and nothing else.
    __adapt(render) {
        renderAdapted = render;
    },
    // For 'react', and for 'rsui/web' giving a page's elements their refs.
    __react: { use, Suspense, attachRef, componentStack, catchWith, useMemoCache },
    // For 'rsui/desktop', and nothing else.
    __host: () => host,
    __passClick: PASS_CLICK,
    // For 'rsui/web': what says what an event happened to, and what hears what `document` and `window` listen for.
    __eventTargets(find) {
        eventTargets = find;
    },
    __eventPath(find) {
        eventPath = find;
    },
    __hearDocument(hear) {
        documentHeard = hear;
    },
    __newEventBase: () => clickEvent,
    __pageAgent: PAGE_AGENT,
    __colorScheme: currentColorScheme,
    // For 'rsui/tauri', which knows which of its windows is rendering by this.
    __contextNow: contextNow,
    // For 'rsui/web', which finds out from it what's around the element being drawn.
    __instanceNow: () => current,
    // And has what a component gives back drawn in another order from now on, until `reorder` gives back what it's given.
    __reorderOutput(instance, reorder) {
        moreOf(instance).reorder = reorder;
        markDirty(instance);
    },
    __provideHere: provideHere,
    __readsHere: readsHere,
    // For 'rsui/web', which draws a page's plain elements in place: see `newFrame`.
    __newFrame: newFrame,
    __renderPlaced: renderPlaced,
    __forgetFrame: forgetFrame,
    __framePending: (frame) => (frame.flags & FORCED) !== 0,
    __markStateSet: (instance) => markStateSet(instance),
    __everything: () => everything,
    set __placedChanged(hear) {
        placedChanged = hear;
    },
    // For 'react-dom', whose `createPortal` into another document's node is this.
    __foreignPortal: (children, container, key) => (foreignContainer(container)
        ? __jsx(ForeignPortal, { container, children, key }, null, key !== undefined && key !== null)
        : pageFragment(container) ? __jsx(FragmentPortal, { children, key }, null, key !== undefined && key !== null) : children),
    // And into the page's `body`: see `PagePortal`.
    __pagePortal: (children, key) => __jsx(PagePortal, { children, key }, null, key !== undefined && key !== null),
    __desktopEvents(hear) {
        desktopEvents = hear;
    },
    // For 'rsui/web', which Rust tells when a window comes to the front or goes behind another, or is minimized or brought back.
    // A real window's state is the system's, which may say so already; one that's only in memory, as in a test, is what this says
    // it is now. Either way what's listening hears it if it's changed.
    __windowChanged(id, changes) {
        const real = host.windowState(id);
        if (real === null || real === undefined) {
            Object.assign(windowKnown(id), changes);
        }
        windowStateChanged(id);
    },
    // For `rsui/dom-globals`' `matchMedia`, which is told when the system goes from light to dark or back.
    __onColorScheme(hear) {
        colorSchemeHeard.add(hear);
    },
    // For `rsui/desktop`'s clipboard, which is told what was pasted.
    __onPasted(hear) {
        pastedTo = hear;
    },
    Fragment,
    useState,
    useReducer,
    useRef,
    useMemo,
    useCallback,
    useEffect,
    useLayoutEffect,
    useInsertionEffect,
    useAnimated,
    Activity,
    focus,
    Window,
    mainWindow,
    useCurrentWindow,
    useWindowState,
    Switch,
    RadioGroup,
    ProgressBar,
    Spinner,
    Tabs,
    Svg,
    Path,
    Line,
    Polyline,
    Polygon,
    Rect,
    Circle,
    Ellipse,
    G,
    useColorScheme,
    Checkbox,
    Slider,
    createContext,
    useContext,
    Dialog,
    MenuButton,
    Select,
    TextInput,
};
