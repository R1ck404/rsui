// Plain function calls and recursion.
function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
function ack(m, n) { return m === 0 ? n + 1 : n === 0 ? ack(m - 1, 1) : ack(m - 1, ack(m, n - 1)); }
function tak(x, y, z) { return y >= x ? z : tak(tak(x - 1, y, z), tak(y - 1, z, x), tak(z - 1, x, y)); }
console.log('fib', fib(29), ack(2, 1500), tak(18, 12, 6));
