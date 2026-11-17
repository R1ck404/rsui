// Property reads at sites that see several shapes (polymorphic) and many shapes (megamorphic).
function make(kind, i) {
    switch (kind) {
        case 0: return { x: i, y: 1 };
        case 1: return { y: 1, x: i };
        case 2: return { x: i, y: 1, z: 2 };
        case 3: return { a: 0, x: i, y: 1 };
        case 4: return { b: 0, c: 0, x: i, y: 1 };
        case 5: { const o = { y: 1 }; o.x = i; return o; }
        case 6: return { x: i, y: 1, w: 'w' };
        default: return { q: null, r: null, s: null, x: i, y: 1 };
    }
}
function makeMany(kind, i) {
    const o = {};
    o['k' + kind] = kind;
    o.x = i;
    o.y = 1;
    return o;
}
const four = [], eight = [], many = [];
for (let i = 0; i < 4000; i++) {
    four.push(make(i % 4, i));
    eight.push(make(i % 8, i));
    many.push(makeMany(i % 24, i));
}
function sumX(list) {
    let s = 0;
    for (let i = 0; i < list.length; i++) s += list[i].x + list[i].y;
    return s;
}
function bump(list) {
    for (let i = 0; i < list.length; i++) list[i].x = list[i].x + 1;
}
let total = 0;
for (let r = 0; r < 200; r++) {
    total += sumX(four) + sumX(eight) + sumX(many);
    bump(four); bump(eight); bump(many);
}
console.log('prop_poly', total);
