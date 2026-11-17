// Spread and destructuring: object spread to update state, array spread, rest parameters, defaults, Object.assign.
function update(state, i) { return { ...state, count: state.count + 1, last: i }; }
function sum(...values) { let s = 0; for (const v of values) s += v; return s; }
function point({ x, y = 0, z = 1 }, [a, b] = [1, 2]) { return x + y * z + a + b; }
let state = { count: 0, last: 0, name: 'state', items: [], flags: { a: true } };
let total = 0;
for (let i = 0; i < 200000; i++) {
    state = update(state, i);
    const { count, last, ...others } = state;
    total += count - last + (others.name.length);
}
for (let i = 0; i < 200000; i++) {
    const list = [i, i + 1, i + 2];
    const more = [...list, ...list, 7];
    const [first, , third, ...tail] = more;
    total += first + third + tail.length + sum(...list);
}
for (let i = 0; i < 200000; i++) {
    total += point({ x: i, y: 2 }) + point({ x: 1 }, [i & 1, 3]);
    let a = i, b = i * 2;
    [a, b] = [b, a];
    total += a - b;
}
for (let i = 0; i < 50000; i++) {
    const merged = Object.assign({}, state, { extra: i });
    total += merged.extra & 15;
}
console.log('spread_destructure', total % 1000000007);
