// What `import ... from 'react'` gives, for code written for React, and the libraries around it, to run on rsui. It's only loaded by
// what asks for it. It's the same hooks and elements as 'rsui', with what React has on top of those made from them.

const rsui = require('rsui');
const { useState, useRef, useEffect, useMemo, useCallback, useReducer, useContext, useLayoutEffect, useInsertionEffect, Activity, Fragment } = rsui;
const { use, Suspense, attachRef, componentStack, catchWith } = rsui.__react;

function memo(component) {
    // Its props are compared one by one, as React's `memo` has it, where another component given props that hold objects is
    // rendered again whenever what it's in is (see `JsProps` in bridge.rs). A comparison of its own is only there to skip work,
    // so going without one can't change what's shown.
    try {
        component.__rsuiShallow = true;
    } catch {
        // Something that can't be told so is rendered again, as anything without `memo` is.
    }
    return component;
}

function forwardRef(render) {
    const forwarded = (props) => {
        const { ref, ...rest } = props;
        return render(rest, ref === undefined ? null : ref);
    };
    // It has no name of its own, as what React gives back has none, so that what looks for `name || displayName` finds what
    // it's for, which is what it's called.
    Object.defineProperty(forwarded, 'name', { value: '', configurable: true });
    forwarded.displayName = render.displayName || render.name;
    return forwarded;
}

function createRef() {
    return { current: null };
}

function useImperativeHandle(ref, make, deps) {
    useLayoutEffect(() => attachRef(ref, make()), deps === undefined ? undefined : [...deps, ref]);
}

/// A function that's always the same, which calls what it was last given, for an effect to call without having to run again
/// whenever that changes, as React 19.2's is.
function useEffectEvent(callback) {
    const held = useRef(null);
    if (held.current === null) {
        const event = { latest: callback, call: null };
        event.call = function (...args) {
            return event.latest.apply(this, args);
        };
        held.current = event;
    }
    held.current.latest = callback;
    return held.current.call;
}

function useSyncExternalStore(subscribe, getSnapshot) {
    const [, redraw] = useState(0);
    const value = getSnapshot();
    const latest = useRef(value);
    latest.current = value;
    const read = useRef(getSnapshot);
    read.current = getSnapshot;
    useEffect(() => {
        const check = () => {
            if (!Object.is(read.current(), latest.current)) {
                redraw((count) => count + 1);
            }
        };
        // It could have changed between rendering and getting here.
        check();
        return subscribe(check);
    }, [subscribe]);
    return value;
}

let nextId = 0;

function useId() {
    const id = useRef(null);
    if (id.current === null) {
        nextId += 1;
        id.current = ':r' + nextId.toString(36) + ':';
    }
    return id.current;
}

// Transitions. Nothing here renders in the background, so what a transition changes is shown as soon as it's done, as anything
// else is. What's left of one is how long it lasts, which for an async function is until it's settled: `useTransition` says
// it's pending until then, and what `useOptimistic` shows for it lasts as long.

// The transition whose function is running now, if one is, which is what an optimistic update made now goes with.
let transition = null;

const isThenable = (value) => value !== null && (typeof value === 'object' || typeof value === 'function') && typeof value.then === 'function';

/// One less thing for a transition to wait for. It's over once there's nothing left.
function release(made) {
    made.waiting -= 1;
    if (made.waiting === 0) {
        made.settled = true;
        const done = made.done;
        made.done = [];
        for (const run of done) {
            run();
        }
    }
}

/// Keeps the transition that's running now going until what's given back is called, for something started in it that finishes
/// later, as an action it dispatched or a transition inside it. React says they're entangled.
function holdTransition() {
    const during = transition;
    if (during === null || during.settled) {
        return () => {};
    }
    during.waiting += 1;
    let held = true;
    return () => {
        if (held) {
            held = false;
            release(during);
        }
    };
}

/// Runs `scope` as a transition, and gives back that transition, with `failed` called if it throws or its promise is rejected.
function runTransition(scope, failed) {
    const made = { settled: false, waiting: 1, done: [holdTransition()], result: undefined };
    const settle = () => release(made);
    const before = transition;
    transition = made;
    let result;
    try {
        result = scope();
    } catch (error) {
        transition = before;
        settle();
        failed(error);
        return made;
    }
    transition = before;
    made.result = result;
    if (isThenable(result)) {
        result.then(settle, (error) => {
            settle();
            failed(error);
        });
    } else {
        settle();
    }
    return made;
}

// What goes wrong in a transition that no component is there to show, as a browser has it.
const reportError = (error) => (typeof globalThis.reportError === 'function' ? globalThis.reportError(error) : console.error(error));

function startTransition(scope) {
    runTransition(scope, reportError);
}

const NOT_PENDING = Object.freeze({ going: 0, failed: null });

/// Whether a transition this started is still going, and what starts one. What goes wrong in one is thrown where this is used, for
/// an error boundary to show, as React 19 does.
function useTransition() {
    const [shown, setShown] = useState(NOT_PENDING);
    const held = useRef(null);
    if (held.current === null) {
        let going = 0;
        held.current = (scope) => {
            const made = runTransition(scope, (error) => setShown({ going, failed: { error } }));
            if (!made.settled) {
                going += 1;
                setShown((now) => ({ going, failed: now.failed }));
                made.done.push(() => {
                    going -= 1;
                    setShown((now) => ({ going, failed: now.failed }));
                });
            }
        };
    }
    if (shown.failed !== null) {
        throw shown.failed.error;
    }
    return [shown.going > 0, held.current];
}

/// What `state` would be with what's been asked for applied to it, until the transition that asked for it is over, by which
/// time `state` itself should say so. Outside of one, it's over as soon as it's asked for, as React says it has to be in one.
function useOptimistic(state, update) {
    const [, redraw] = useState(0);
    const held = useRef(null);
    if (held.current === null) {
        const asked = [];
        held.current = {
            asked,
            add(action) {
                const during = transition;
                if (during === null || during.settled) {
                    console.error('An optimistic state update occurred outside a transition or action, so it was undone straight away. '
                        + 'Make it in an action, or in startTransition.');
                    return;
                }
                const one = { action };
                asked.push(one);
                redraw((count) => count + 1);
                during.done.push(() => {
                    asked.splice(asked.indexOf(one), 1);
                    redraw((count) => count + 1);
                });
            },
        };
    }
    let shown = state;
    for (const one of held.current.asked) {
        shown = typeof update === 'function' ? update(shown, one.action) : one.action;
    }
    return [shown, held.current.add];
}

/// State that an action makes: `dispatch(payload)` calls `action(state, payload)`, and what it gives back, or what its promise
/// gives, is the state from then on. Each waits for the one before, and it's pending until they're all done. What goes wrong in
/// one is thrown where this is used, for an error boundary, and what was still to run doesn't run.
function useActionState(action, initial) {
    const [shown, setShown] = useState(() => ({ state: initial, pending: false, failed: null }));
    const held = useRef(null);
    if (held.current === null) {
        const queue = [];
        const actions = { action, state: initial, running: false, dispatch: null };
        const fail = (error) => {
            for (const waiting of queue) {
                waiting.release();
            }
            queue.length = 0;
            actions.running = false;
            setShown({ state: actions.state, pending: false, failed: { error } });
        };
        const next = () => {
            if (queue.length === 0) {
                actions.running = false;
                setShown({ state: actions.state, pending: false, failed: null });
                return;
            }
            const { payload, release } = queue.shift();
            let failed = false;
            const made = runTransition(() => actions.action(actions.state, payload), (error) => {
                failed = true;
                release();
                fail(error);
            });
            if (failed) {
                return;
            }
            const done = (value) => {
                actions.state = value;
                release();
                if (queue.length > 0) {
                    setShown({ state: value, pending: true, failed: null });
                }
                next();
            };
            if (isThenable(made.result)) {
                // A rejected one has already been dealt with by `fail`.
                made.result.then(done, () => {});
            } else {
                done(made.result);
            }
        };
        actions.dispatch = (payload) => {
            // A transition it's dispatched in lasts until it's done, though it might wait for others before it starts.
            queue.push({ payload, release: holdTransition() });
            if (!actions.running) {
                actions.running = true;
                setShown((now) => ({ state: now.state, pending: true, failed: null }));
                next();
            }
        };
        held.current = actions;
    }
    held.current.action = action;
    if (shown.failed !== null) {
        throw shown.failed.error;
    }
    return [shown.state, held.current.dispatch, shown.pending];
}

/// `value`, as nothing renders in the background to wait for it. Given `initialValue`, that's what it is the first time, and it's
/// `value` from the render straight after, as in React 19.
function useDeferredValue(value, initialValue) {
    const [started, setStarted] = useState(initialValue === undefined);
    useEffect(() => {
        if (!started) {
            setStarted(true);
        }
    }, [started]);
    return started ? value : initialValue;
}

const useDebugValue = () => {};

function createContext(fallback) {
    const context = rsui.createContext(fallback);
    context.Consumer = function Consumer({ children }) {
        return children(useContext(context));
    };
    return context;
}

function isValidElement(value) {
    return typeof value === 'object' && value !== null && 'type' in value && 'kids' in value;
}

function createElement(type, props, ...kids) {
    // One that says `native` is rsui's own, in the platform's look.
    if (typeof type === 'string' && globalThis.__rsuiDom !== undefined && (props == null || props.native !== true)) {
        type = globalThis.__rsuiDom(type) ?? type;
    }
    // Its props are always an object of its own, copied from what it's given, as React's `createElement` makes them: what's given
    // can be the same object each time (TanStack Table's `flexRender(def.cell, cell.getContext())` gives a memoized context), and a
    // component given the same props object as before isn't rendered again, so the row's checkbox kept its old state.
    let given = props === null || props === undefined ? {} : { ...props };
    if (typeof type !== 'function' || type === Fragment) {
        // What's built in is drawn from what's in it apart from its props, and its `props.children` says the same, as React's does.
        if (kids.length > 0) {
            given.children = kids.length === 1 ? kids[0] : kids;
        } else if (given.children !== undefined) {
            kids = Array.isArray(given.children) ? given.children : [given.children];
        }
    }
    return __jsx(type, given, kids, 'key' in given);
}

function cloneElement(element, props, ...kids) {
    const merged = { ...element.props, ...props };
    const key = props && 'key' in props ? props.key : element.key;
    if (key !== undefined && key !== null) {
        merged.key = key;
    }
    // New children can come after its props, or as one of them, and they're in its `props.children` either way, as React has it.
    if (kids.length > 0) {
        merged.children = kids.length === 1 ? kids[0] : kids;
    }
    if (typeof element.type === 'function' && element.type !== Fragment) {
        return __jsx(element.type, merged, undefined, 'key' in merged);
    }
    const children = 'children' in merged ? merged.children : undefined;
    const given = children === undefined ? element.kids : Array.isArray(children) ? children : [children];
    return __jsx(element.type, merged, given, 'key' in merged);
}

/// What makes elements of one type, as React before 19 has it: `createFactory(Box)(props, ...children)`.
function createFactory(type) {
    const factory = (props, ...kids) => createElement(type, props, ...kids);
    factory.type = type;
    return factory;
}

function flatten(children, out) {
    if (Array.isArray(children)) {
        for (const child of children) {
            flatten(child, out);
        }
    } else if (children !== null && children !== undefined && typeof children !== 'boolean') {
        out.push(children);
    }
    return out;
}

const Children = {
    toArray: (children) => flatten(children, []),
    count: (children) => flatten(children, []).length,
    map: (children, run, self) => (children === null || children === undefined ? children : flatten(children, []).map((child, at) => run.call(self, child, at))),
    forEach: (children, run, self) => {
        flatten(children, []).forEach((child, at) => run.call(self, child, at));
    },
    only(children) {
        if (!isValidElement(children)) {
            throw new Error('React.Children.only expected to receive a single React element child.');
        }
        return children;
    },
};

// These are plain functions, and not classes, for what was compiled to call them with `Component.call(this, props)`.
function Component(props, context) {
    this.props = props;
    this.context = context;
    this.refs = {};
}
Component.prototype.isReactComponent = {};
Component.prototype.setState = function (change, done) {
    this.__update(change, done);
};
Component.prototype.forceUpdate = function (done) {
    this.__update(null, done);
};

function PureComponent(props, context) {
    Component.call(this, props, context);
}
PureComponent.prototype = Object.create(Component.prototype);
PureComponent.prototype.constructor = PureComponent;
PureComponent.prototype.isPureReactComponent = true;

function withDefaults(defaults, props) {
    const merged = { ...props };
    for (const name in defaults) {
        if (merged[name] === undefined) {
            merged[name] = defaults[name];
        }
    }
    return merged;
}

function sameProps(a, b) {
    if (a === b) {
        return true;
    }
    if (a === null || b === null || a === undefined || b === undefined) {
        return false;
    }
    const names = Object.keys(a);
    return names.length === Object.keys(b).length && names.every((name) => Object.is(a[name], b[name]));
}

/// What a class component comes to, as hooks.
function renderClass(type, props, owner) {
    const [, redraw] = useState(0);
    const held = useRef(null);
    const snapshot = useRef(undefined);
    const given = type.contextType ? useContext(type.contextType) : undefined;
    let merged = type.defaultProps ? withDefaults(type.defaultProps, props) : props;
    const ref = merged.ref;
    if (ref !== undefined) {
        const { ref: _, ...rest } = merged;
        merged = rest;
    }
    const derive = (instance) => {
        if (type.getDerivedStateFromProps) {
            const change = type.getDerivedStateFromProps(merged, instance.state);
            if (change !== null && change !== undefined) {
                instance.state = { ...instance.state, ...change };
            }
        }
    };
    const first = held.current === null;
    if (first) {
        const instance = new type(merged, given);
        const made = { instance, after: [], caught: [], forced: false, rendering: true, output: null, shownState: undefined };
        instance.props = merged;
        instance.context = given;
        instance.__update = (change, done) => {
            if (change === null) {
                made.forced = true;
            } else {
                const next = typeof change === 'function' ? change(instance.state, instance.props) : change;
                if (next === null || next === undefined) {
                    return;
                }
                instance.state = { ...instance.state, ...next };
            }
            if (done) {
                made.after.push(done);
            }
            // While it's rendering, what this does is already part of that.
            if (!made.rendering) {
                redraw((count) => count + 1);
            }
        };
        if (type.getDerivedStateFromError || instance.componentDidCatch) {
            catchWith(owner, (error, info) => {
                if (type.getDerivedStateFromError) {
                    instance.state = { ...instance.state, ...type.getDerivedStateFromError(error) };
                }
                made.caught.push({ error, info: info ?? { componentStack: componentStack(owner) } });
                made.forced = true;
                redraw((count) => count + 1);
            });
        }
        derive(instance);
        const willMount = instance.UNSAFE_componentWillMount ?? instance.componentWillMount;
        willMount?.call(instance);
        held.current = made;
    }
    const made = held.current;
    const instance = made.instance;
    const before = { props: instance.props, state: made.shownState };
    let updated = first;
    made.rendering = true;
    if (first) {
        made.output = instance.render();
    } else {
        const willReceive = instance.UNSAFE_componentWillReceiveProps ?? instance.componentWillReceiveProps;
        if (willReceive && before.props !== merged) {
            willReceive.call(instance, merged, given);
        }
        derive(instance);
        // These look at what's about to change from where it still was.
        const next = instance.state;
        instance.state = before.state;
        updated = made.forced;
        if (!updated) {
            if (instance.shouldComponentUpdate) {
                updated = instance.shouldComponentUpdate(merged, next, given) !== false;
            } else {
                updated = !type.prototype.isPureReactComponent || !sameProps(before.props, merged) || !sameProps(before.state, next);
            }
        }
        if (updated) {
            const willUpdate = instance.UNSAFE_componentWillUpdate ?? instance.componentWillUpdate;
            willUpdate?.call(instance, merged, next, given);
        }
        instance.props = merged;
        instance.context = given;
        instance.state = next;
        if (updated) {
            made.output = instance.render();
            if (instance.getSnapshotBeforeUpdate) {
                snapshot.current = instance.getSnapshotBeforeUpdate(before.props, before.state);
            }
        }
    }
    made.forced = false;
    made.rendering = false;
    made.shownState = instance.state;
    // These are run as layout effects are, as React has them, before what changed is drawn.
    useLayoutEffect(() => attachRef(ref, instance), [ref]);
    useLayoutEffect(() => {
        instance.componentDidMount?.();
        return () => instance.componentWillUnmount?.();
    }, []);
    useLayoutEffect(() => {
        if (updated && !first) {
            instance.componentDidUpdate?.(before.props, before.state, snapshot.current);
        }
        const caught = made.caught;
        made.caught = [];
        for (const { error, info } of caught) {
            instance.componentDidCatch?.(error, info);
        }
        const after = made.after;
        made.after = [];
        for (const done of after) {
            done.call(instance);
        }
    });
    return made.output;
}

function renderAdapted(type, props, owner) {
    if (type.prototype?.isReactComponent !== undefined) {
        return renderClass(type, props, owner);
    }
    return type(withDefaults(type.defaultProps, props));
}

rsui.__adapt(renderAdapted);

/// A component that's loaded the first time it's rendered. The `Suspense` around it shows its fallback until it's there.
function lazy(load) {
    let loading = null;
    return function Lazy(props) {
        loading ??= load();
        return __jsx(use(loading).default, props, undefined, false);
    };
}

const StrictMode = Fragment;
const Profiler = Fragment;

/// What React gives a server component to share what a call worked out with others that make it during one request. There's no
/// request here, so it's the function itself, called each time, as React's is outside of one.
function cache(run) {
    return function (...args) {
        return run.apply(this, args);
    };
}

/// What says that a server's request is over, which there's never any of here.
const cacheSignal = () => null;

const react = {
    Activity,
    Children,
    Component,
    Fragment,
    Profiler,
    PureComponent,
    StrictMode,
    Suspense,
    cache,
    cacheSignal,
    cloneElement,
    createContext,
    createElement,
    createFactory,
    createRef,
    forwardRef,
    isValidElement,
    lazy,
    memo,
    startTransition,
    use,
    useActionState,
    useCallback,
    useContext,
    useDebugValue,
    useDeferredValue,
    useEffect,
    useEffectEvent,
    useId,
    useImperativeHandle,
    useInsertionEffect,
    useLayoutEffect,
    useMemo,
    useOptimistic,
    useReducer,
    useRef,
    useState,
    useSyncExternalStore,
    useTransition,
    version: '19.2.0',
};
react.default = react;
module.exports = react;
