// Arrays the plain way: push, pop, index reads and writes, indexOf/includes, and for-of.
const N = 400000;
let total = 0;
for (let r = 0; r < 4; r++) {
    const a = [];
    for (let i = 0; i < N; i++) a.push(i * 3);
    for (let i = 0; i < N; i++) a[i] = a[i] + (a[N - 1 - i] & 7);
    let s = 0;
    for (let i = 0; i < a.length; i++) s += a[i];
    for (const v of a) s -= v >> 1;
    while (a.length > N / 2) s += a.pop() & 15;
    total = (total + s) % 1000000007;
}
const small = [];
for (let i = 0; i < 200; i++) small.push(i * 7 % 200);
for (let r = 0; r < 3000; r++) {
    total += small.indexOf(r % 250) + (small.includes(r % 300) ? 1 : 0);
}
const grid = [];
for (let y = 0; y < 300; y++) {
    const row = new Array(300);
    for (let x = 0; x < 300; x++) row[x] = (x * y) & 255;
    grid.push(row);
}
for (let r = 0; r < 4; r++) {
    for (let y = 1; y < 299; y++) {
        const above = grid[y - 1], row = grid[y], below = grid[y + 1];
        for (let x = 1; x < 299; x++) row[x] = (above[x] + below[x] + row[x - 1] + row[x + 1]) >> 2;
    }
}
for (let y = 0; y < 300; y++) total += grid[y][y];
console.log('array_ops', total);
