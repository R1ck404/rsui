// Property reads and writes at sites that only ever see one shape of object.
class Vec {
    constructor(x, y, z) { this.x = x; this.y = y; this.z = z; }
}
function Particle(i) {
    this.pos = new Vec(i, i * 2, i * 3);
    this.vel = new Vec(1, -1, 0.5);
    this.mass = 1 + (i % 5);
    this.alive = true;
}
const list = [];
for (let i = 0; i < 2000; i++) list.push(new Particle(i));
let total = 0;
for (let step = 0; step < 500; step++) {
    for (let i = 0; i < list.length; i++) {
        const p = list[i];
        const pos = p.pos, vel = p.vel;
        pos.x += vel.x * p.mass;
        pos.y += vel.y * p.mass;
        pos.z += vel.z;
        if (pos.y < -1000) { vel.y = -vel.y; p.alive = !p.alive; }
        if (p.alive) total += pos.x - pos.y;
    }
}
const config = { width: 10, height: 20, depth: 3 };
let area = 0;
for (let i = 0; i < 1500000; i++) area += config.width * config.height + config.depth;
console.log('prop_mono', total, area);
