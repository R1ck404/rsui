// Classes and prototype chains: subclasses with super calls, inherited methods found several levels up, instanceof,
// static members, private fields, and Object.create chains.
class Shape {
    constructor(name) { this.name = name; }
    describe() { return this.name.length + this.area(); }
    area() { return 0; }
    static count = 0;
}
class Rect extends Shape {
    constructor(w, h) { super('rect'); this.w = w; this.h = h; Shape.count++; }
    area() { return this.w * this.h; }
}
class Square extends Rect {
    constructor(s) { super(s, s); this.name = 'square'; }
    area() { return super.area() + 1; }
}
class Cube extends Square {
    #depth;
    constructor(s) { super(s); this.#depth = s; }
    volume() { return this.area() * this.#depth; }
}
let total = 0;
for (let i = 0; i < 200000; i++) {
    const shape = i % 3 === 0 ? new Rect(i & 7, 3) : i % 3 === 1 ? new Square(i & 15) : new Cube(i & 3);
    total += shape.describe();
    if (shape instanceof Square) total++;
    if (shape instanceof Cube) total += shape.volume();
}
const base = { greet() { return 1; }, level: 0 };
let proto = base;
for (let i = 0; i < 6; i++) proto = Object.create(proto, { ['own' + i]: { value: i, enumerable: true } });
for (let i = 0; i < 1500000; i++) total += proto.greet() + proto.level;
console.log('class_proto', total, Shape.count);
