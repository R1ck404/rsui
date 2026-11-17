// What compiled JSX calls in a package that was built for React's newer way of doing it: `react/jsx-runtime`.

const { Fragment } = require('rsui');

function jsx(type, props, key) {
    if (typeof type === 'string' && globalThis.__rsuiDom !== undefined) {
        type = globalThis.__rsuiDom(type) ?? type;
    }
    // What it's given is its own, made for this call, and it keeps its `children` there as React's does.
    const given = key === undefined ? props : { ...props, key };
    if (typeof type === 'function' && type !== Fragment) {
        return __jsx(type, given, undefined, key !== undefined);
    }
    const children = props.children;
    const kids = children === undefined ? [] : Array.isArray(children) ? children : [children];
    return __jsx(type, given, kids, key !== undefined);
}

module.exports = { Fragment, jsx, jsxs: jsx, jsxDEV: jsx };
