// What a React-like UI does each frame, in plain JS: function components with hook-style state, a fresh tree of small
// element objects and children arrays every frame, then a keyed diff against the previous frame that counts the patches.
let hookStates = new Map();
let currentPath = '';
let hookIndex = 0;
function useState(initial) {
    const key = currentPath + '#' + hookIndex++;
    if (!hookStates.has(key)) hookStates.set(key, initial);
    return [hookStates.get(key), (v) => hookStates.set(key, v)];
}
function useMemo(make, deps) {
    const key = currentPath + '#' + hookIndex++;
    const old = hookStates.get(key);
    if (old && old.deps.length === deps.length && old.deps.every((d, i) => d === deps[i])) return old.value;
    const value = make();
    hookStates.set(key, { deps, value });
    return value;
}
function h(type, props, ...children) {
    return { type, props: props || {}, key: props && props.key !== undefined ? props.key : null, children: children.flat() };
}
function render(node, path) {
    if (node === null || node === undefined || typeof node !== 'object') return node;
    if (typeof node.type === 'function') {
        const saved = currentPath, savedIndex = hookIndex;
        currentPath = path + '/' + node.type.name + (node.key !== null ? ':' + node.key : '');
        hookIndex = 0;
        const out = render(node.type({ ...node.props, children: node.children }), currentPath);
        currentPath = saved; hookIndex = savedIndex;
        return out;
    }
    const children = [];
    for (let i = 0; i < node.children.length; i++) {
        const child = node.children[i];
        children.push(render(child, path + '/' + (child && child.key !== null && child.key !== undefined ? child.key : i)));
    }
    return { type: node.type, props: node.props, key: node.key, children };
}
function diff(a, b, patches) {
    if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) {
        if (a !== b) patches.push({ op: 'text', value: b });
        return;
    }
    if (a.type !== b.type) { patches.push({ op: 'replace', node: b }); return; }
    const keys = new Set([...Object.keys(a.props), ...Object.keys(b.props)]);
    for (const k of keys) {
        if (k === 'children' || typeof b.props[k] === 'function') continue;
        if (a.props[k] !== b.props[k]) patches.push({ op: 'prop', key: k, value: b.props[k] });
    }
    const oldByKey = new Map();
    a.children.forEach((c, i) => oldByKey.set(c && c.key != null ? c.key : 'i' + i, c));
    b.children.forEach((c, i) => {
        const key = c && c.key != null ? c.key : 'i' + i;
        if (oldByKey.has(key)) { diff(oldByKey.get(key), c, patches); oldByKey.delete(key); }
        else patches.push({ op: 'insert', key });
    });
    for (const key of oldByKey.keys()) patches.push({ op: 'remove', key });
}

function Row({ item, selected, onSelect }) {
    const [hover] = useState(false);
    const label = useMemo(() => `${item.name} — ${item.amount.toFixed(2)}`, [item.name, item.amount]);
    return h('item', { key: item.id, background: selected ? '#ddf4ff' : hover ? '#eee' : '#fff', onClick: () => onSelect(item.id) },
        h('text', { bold: selected }, label),
        h('text', { color: item.amount > 500 ? 'red' : 'black' }, String(item.count)));
}
function Toolbar({ count, filter }) {
    return h('row', { gap: 8 },
        h('button', { key: 'add' }, 'Add'),
        h('input', { key: 'filter', value: filter }),
        h('text', null, `${count} rows`));
}
function App({ items, selected, filter }) {
    const shown = items.filter((it) => it.name.includes(filter));
    return h('window', { title: 'Bench' },
        h(Toolbar, { count: shown.length, filter }),
        h('list', null, ...shown.map((item) => h(Row, { key: item.id, item, selected: item.id === selected, onSelect: () => {} }))));
}

let items = [];
for (let i = 0; i < 400; i++) items.push({ id: 'r' + i, name: 'Row ' + i, amount: (i * 37) % 1000, count: i });
let previous = null;
let total = 0;
for (let frame = 0; frame < 60; frame++) {
    if (frame % 3 === 0) items = items.map((it, i) => (i % 17 === frame % 17 ? { ...it, amount: it.amount + 1, count: it.count + 1 } : it));
    if (frame % 10 === 5) items = [items[items.length - 1], ...items.slice(0, -1)];
    const tree = render(h(App, { items, selected: 'r' + (frame * 7 % 400), filter: frame % 20 < 10 ? '' : '1' }), '');
    if (previous !== null) {
        const patches = [];
        diff(previous, tree, patches);
        total += patches.length;
    }
    previous = tree;
}
console.log('react_render', total, hookStates.size);
