// Integer arithmetic in tight loops: adds, multiplies, shifts, bitwise ops, modulo and comparisons.
function lcg(n) {
    let a = 1, b = 0, c = 0;
    for (let i = 0; i < n; i++) {
        a = (a * 1664525 + 1013904223) >>> 0;
        b = (b + (i ^ (i >> 3))) | 0;
        c = (c + (i % 7) * (a & 255)) | 0;
    }
    return (a ^ b ^ c) >>> 0;
}
function sieve(n) {
    const flags = [];
    for (let i = 0; i <= n; i++) flags.push(true);
    let count = 0;
    for (let i = 2; i <= n; i++) {
        if (flags[i]) {
            count++;
            for (let k = i + i; k <= n; k += i) flags[k] = false;
        }
    }
    return count;
}
function collatz(limit) {
    let longest = 0, best = 0;
    for (let start = 1; start < limit; start++) {
        let n = start, steps = 0;
        while (n !== 1) {
            n = (n & 1) === 0 ? n / 2 : 3 * n + 1;
            steps++;
        }
        if (steps > longest) { longest = steps; best = start; }
    }
    return best * 1000 + longest;
}
let sum = 0;
sum = (sum + lcg(3000000)) >>> 0;
sum = (sum + sieve(600000)) >>> 0;
sum = (sum + collatz(60000)) >>> 0;
console.log('int_loop', sum);
