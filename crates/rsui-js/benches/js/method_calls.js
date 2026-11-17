// Method calls: on one class, through getters and setters, and virtual dispatch across a few classes.
class Counter {
    constructor() { this.n = 0; this._step = 1; }
    inc() { this.n += this._step; return this; }
    add(k) { this.n += k; return this.n; }
    get step() { return this._step; }
    set step(v) { this._step = v; }
}
class Circle { constructor(r) { this.r = r; } area() { return 3 * this.r * this.r; } }
class Square { constructor(s) { this.s = s; } area() { return this.s * this.s; } }
class Tri { constructor(b, h) { this.b = b; this.h = h; } area() { return (this.b * this.h) >> 1; } }
const c = new Counter();
for (let i = 0; i < 2000000; i++) {
    c.inc();
    c.add(i & 3);
    if ((i & 1023) === 0) c.step = c.step === 1 ? 2 : 1;
}
const shapes = [];
for (let i = 0; i < 300; i++) shapes.push(i % 3 === 0 ? new Circle(i) : i % 3 === 1 ? new Square(i) : new Tri(i, i + 1));
let area = 0;
for (let r = 0; r < 2500; r++) {
    for (let i = 0; i < shapes.length; i++) area += shapes[i].area();
}
console.log('method_calls', c.n, area);
