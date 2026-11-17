// Closures: making them in a loop, calling them, and writing variables they've captured.
function makeAdder(k) { return (x) => x + k; }
function makeCounter() {
    let count = 0;
    return { inc() { count++; return count; }, get() { return count; } };
}
function compose(f, g) { return (x) => f(g(x)); }
let total = 0;
for (let i = 0; i < 300000; i++) {
    const add = makeAdder(i & 15);
    total += add(i) & 0xffff;
}
const counter = makeCounter();
for (let i = 0; i < 1000000; i++) counter.inc();
total += counter.get();
const pipeline = compose(makeAdder(1), compose(makeAdder(2), (x) => x * 2));
for (let i = 0; i < 500000; i++) total = (total + pipeline(i)) % 1000000007;
const fns = [];
for (let i = 0; i < 1000; i++) fns.push(() => i * 2);
for (let r = 0; r < 300; r++) for (let i = 0; i < fns.length; i++) total = (total + fns[i]()) % 1000000007;
console.log('closures', total);
