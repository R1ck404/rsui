// Generators and iterators: generator pipelines, for-of over them, spreading one, and a hand-written iterator.
function* range(n) { for (let i = 0; i < n; i++) yield i; }
function* map(it, f) { for (const v of it) yield f(v); }
function* filter(it, f) { for (const v of it) if (f(v)) yield v; }
function* take(it, n) { if (n <= 0) return; for (const v of it) { yield v; if (--n <= 0) return; } }
class Countdown {
    constructor(n) { this.n = n; }
    [Symbol.iterator]() {
        let n = this.n;
        return { next: () => (n > 0 ? { value: n--, done: false } : { value: undefined, done: true }) };
    }
}
let total = 0;
for (let r = 0; r < 10; r++) {
    for (const v of take(filter(map(range(100000), (x) => x * 3), (x) => x % 2 === 0), 40000)) total += v & 0xff;
    total += [...range(2000)].length;
    for (const v of new Countdown(30000)) total += v & 1;
    const [a, b, ...rest] = range(500);
    total += a + b + rest.length;
}
console.log('generators', total);
