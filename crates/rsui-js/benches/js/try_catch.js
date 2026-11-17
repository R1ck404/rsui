// Exceptions: try blocks that don't throw in a hot loop, throwing and catching Errors and plain values, and finally.
function risky(i) {
    if (i % 100 === 0) throw new Error('bad ' + i);
    return i & 3;
}
function plain(i) {
    if (i % 10 === 0) throw i;
    return 1;
}
let total = 0;
for (let i = 0; i < 3000000; i++) {
    try { total += i & 1; } catch (e) { total -= 1; }
}
for (let i = 0; i < 400000; i++) {
    try { total += risky(i); } catch (e) { total += e.message.length; }
}
for (let i = 0; i < 400000; i++) {
    try { total += plain(i); } catch (e) { total += e & 7; } finally { total++; }
}
class ParseError extends Error { constructor(at) { super('parse error'); this.at = at; } }
for (let i = 0; i < 40000; i++) {
    try { throw new ParseError(i); } catch (e) { if (e instanceof ParseError) total += e.at & 1; }
}
console.log('try_catch', total);
