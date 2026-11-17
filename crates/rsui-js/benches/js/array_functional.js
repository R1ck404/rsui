// Array methods with callbacks: map, filter, reduce, forEach, some, every, find, findIndex, flatMap.
const items = [];
for (let i = 0; i < 10000; i++) items.push({ id: i, price: (i * 37) % 1000, qty: i % 9, tag: i % 3 === 0 ? 'a' : 'b' });
let total = 0;
for (let r = 0; r < 60; r++) {
    const prices = items.map((it) => it.price * it.qty);
    const big = prices.filter((p) => p > 2000);
    total += big.reduce((sum, p) => sum + p, 0);
    let count = 0;
    items.forEach((it) => { if (it.tag === 'a') count++; });
    total += count;
    total += items.some((it) => it.price === 999 - r) ? 1 : 0;
    total += items.every((it) => it.qty < 9) ? 1 : 0;
    const found = items.find((it) => it.price === r * 10);
    total += found ? found.id : -1;
    total += items.findIndex((it) => it.id === 5000 + r);
    total += items.slice(0, 200).flatMap((it) => [it.qty, it.qty]).length;
    total = total % 1000000007;
}
const numbers = Array.from({ length: 50000 }, (_, i) => i);
for (let r = 0; r < 15; r++) {
    total = (total + numbers.map((n) => n * 2).filter((n) => n % 3 === 0).reduce((a, b) => a + b, 0)) % 1000000007;
}
console.log('array_functional', total);
