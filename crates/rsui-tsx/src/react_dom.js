// What `import ... from 'react-dom'` gives. There's no page here, so this is only what libraries call along the way.

const rsui = require('rsui');
const { use } = rsui.__react;

const notHere = (what) => () => {
    throw new Error(what + " isn't how an app starts on rsui. Its main file's default export is its root component.");
};

// The form something's in, which `useFormStatus` reads in what's inside it: 'rsui/web' gives a page's `<form>` to what's in it, as
// what to `subscribe` to and its `status`. While its `action` runs, that's what it sent, and how, with `pending` true, and this is
// what there is otherwise. Outside a form, it's null.
const NOT_SENDING = Object.freeze({ pending: false, data: null, method: null, action: null });
const FormStatus = rsui.createContext(null);
FormStatus.Provider.displayName = 'FormContext';
const NO_FORM = { subscribe: () => () => {}, getStatus: () => NOT_SENDING };

function useFormStatus() {
    const form = use(FormStatus);
    const store = form?.subscribe === undefined ? NO_FORM : form;
    return require('react').useSyncExternalStore(store.subscribe, store.getStatus);
}

// What says what's going to be needed, so that a browser can start getting it. Everything an app has is here already, so there's
// nothing to start.
const hint = () => {};

// What says to load a script or a stylesheet and run it or apply it straight away, which only a page's `<script>` and `<link>`
// can do. That's said once, rather than left to go unnoticed.
const warned = new Set();
function preinit(href) {
    if (!warned.has(href)) {
        warned.add(href);
        console.warn('react-dom: preinit(' + JSON.stringify(href) + ") can't load anything here, so it's left out");
    }
}

const reactDom = {
    // What's in a portal shows where it's written. An overlay prop on it, a Dialog or a Window, is what takes it out of there.
    // Into another document's node, as React Aria builds its collections, it's put in that, as react-dom does.
    // Into the page's `body`, what's in it is at the end of `body`, as far as the DOM says, as Radix's and every other library's
    // overlays are there.
    createPortal: (children, container, key) => {
        const page = globalThis.document;
        if (page !== undefined && page !== null && (container === page.body || container === page.documentElement)) {
            return rsui.__pagePortal(children, key);
        }
        // Into an element of the page, as Radix Select's value text goes into its trigger: drawn in that element.
        // Only text, which needs nothing of where it's written: elements are drawn where they're written, with their context.
        const textOnly = (kids) => typeof kids === 'string' || typeof kids === 'number' || (Array.isArray(kids) && kids.length > 0 && kids.every(textOnly));
        if (container !== null && typeof container === 'object' && container.__live !== undefined && container.__live.domKey !== undefined && textOnly(children)) {
            return require('rsui/web').__elementPortal(children, container, key);
        }
        return rsui.__foreignPortal(children, container, key);
    },
    flushSync: (run) => run(),
    unstable_batchedUpdates: (run, value) => run(value),
    findDOMNode: () => null,
    render: notHere('ReactDOM.render'),
    hydrate: notHere('ReactDOM.hydrate'),
    createRoot: notHere('createRoot'),
    hydrateRoot: notHere('hydrateRoot'),
    useFormStatus,
    preconnect: hint,
    prefetchDNS: hint,
    preload: hint,
    preloadModule: hint,
    preinit,
    preinitModule: preinit,
    // For 'rsui/web', which gives it to what's in a form while the form's `action` runs.
    __FormStatus: FormStatus,
    version: '19.2.0',
};
reactDom.default = reactDom;
module.exports = reactDom;
