// Promises and async functions: awaiting in a loop, chains of then, Promise.all, and an async generator.
async function leaf(i) { return i & 7; }
async function middle(i) { const a = await leaf(i); const b = await leaf(i + 1); return a + b; }
async function* ticks(n) { for (let i = 0; i < n; i++) yield i; }
async function main() {
    let total = 0;
    for (let i = 0; i < 60000; i++) total += await middle(i);
    let p = Promise.resolve(0);
    for (let i = 0; i < 40000; i++) p = p.then((v) => v + (i & 3));
    total += await p;
    for (let r = 0; r < 40; r++) {
        const batch = [];
        for (let i = 0; i < 500; i++) batch.push(middle(i + r));
        const results = await Promise.all(batch);
        total += results.length + results[r];
    }
    for await (const t of ticks(30000)) total += t & 1;
    try { await Promise.reject(new Error('no')); } catch (e) { total += e.message.length; }
    return total;
}
main().then((total) => console.log('async_await', total));
