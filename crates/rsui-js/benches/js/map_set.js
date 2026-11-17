// Map and Set: number, string and object keys; add, get, has, delete, and iterating.
let total = 0;
for (let r = 0; r < 4; r++) {
    const byNumber = new Map();
    for (let i = 0; i < 50000; i++) byNumber.set(i * 7, i);
    for (let i = 0; i < 100000; i++) { const v = byNumber.get(i); if (v !== undefined) total += v; }
    for (let i = 0; i < 50000; i += 3) byNumber.delete(i * 7);
    for (const [k, v] of byNumber) total = (total + k - v) % 1000000007;
    const byName = new Map();
    for (let i = 0; i < 30000; i++) byName.set('key' + i, i);
    for (let i = 0; i < 30000; i++) total += byName.get('key' + ((i * 13) % 30000));
    const seen = new Set();
    for (let i = 0; i < 80000; i++) seen.add((i * 31) % 20011);
    total += seen.size;
    for (let i = 0; i < 20011; i += 2) if (seen.has(i)) total++;
    const objects = [];
    for (let i = 0; i < 5000; i++) objects.push({ i });
    const meta = new Map();
    for (const o of objects) meta.set(o, o.i * 2);
    for (const o of objects) total += meta.get(o);
    const weak = new WeakMap();
    for (const o of objects) weak.set(o, o.i);
    for (let i = 0; i < objects.length; i += 5) total += weak.get(objects[i]);
    total = total % 1000000007;
}
console.log('map_set', total);
