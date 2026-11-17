// Allocation and the collector: binary trees, and many short-lived small objects and arrays.
function Node(left, right) { this.left = left; this.right = right; }
function make(depth) { return depth === 0 ? new Node(null, null) : new Node(make(depth - 1), make(depth - 1)); }
function check(node) { return node.left === null ? 1 : 1 + check(node.left) + check(node.right); }
let total = 0;
const longLived = make(14);
for (let depth = 4; depth <= 14; depth += 2) {
    const iterations = 1 << (14 - depth + 3);
    for (let i = 0; i < iterations; i++) total += check(make(depth));
}
total += check(longLived);
for (let i = 0; i < 300000; i++) {
    const p = { x: i, y: i + 1, tag: 'p' };
    const pair = [p, { x: p.y, y: p.x }];
    total += pair[1].x - pair[0].x;
}
console.log('alloc_churn', total);
