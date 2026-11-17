// A small ray tracer: vector classes with methods, spheres and a plane, shadows and one reflection. Lots of short-lived
// objects and float math. The picture is reduced to a checksum of its pixels.
class V {
    constructor(x, y, z) { this.x = x; this.y = y; this.z = z; }
    add(o) { return new V(this.x + o.x, this.y + o.y, this.z + o.z); }
    sub(o) { return new V(this.x - o.x, this.y - o.y, this.z - o.z); }
    mul(k) { return new V(this.x * k, this.y * k, this.z * k); }
    dot(o) { return this.x * o.x + this.y * o.y + this.z * o.z; }
    len() { return Math.sqrt(this.dot(this)); }
    norm() { return this.mul(1 / this.len()); }
}
class Sphere {
    constructor(center, radius, color, shine) { this.center = center; this.r2 = radius * radius; this.color = color; this.shine = shine; }
    hit(origin, dir) {
        const oc = origin.sub(this.center);
        const b = oc.dot(dir);
        const c = oc.dot(oc) - this.r2;
        const disc = b * b - c;
        if (disc < 0) return -1;
        const t = -b - Math.sqrt(disc);
        return t > 1e-4 ? t : -1;
    }
    normal(p) { return p.sub(this.center).norm(); }
}
class Plane {
    constructor(y, color) { this.y = y; this.color = color; this.shine = 0.2; }
    hit(origin, dir) {
        if (Math.abs(dir.y) < 1e-6) return -1;
        const t = (this.y - origin.y) / dir.y;
        return t > 1e-4 ? t : -1;
    }
    normal() { return new V(0, 1, 0); }
}
const scene = [
    new Sphere(new V(0, 1, 5), 1, new V(0.9, 0.2, 0.2), 0.5),
    new Sphere(new V(-2, 0.7, 6), 0.7, new V(0.2, 0.9, 0.2), 0.3),
    new Sphere(new V(2, 0.8, 4.5), 0.8, new V(0.2, 0.3, 0.9), 0.6),
    new Plane(0, new V(0.8, 0.8, 0.8)),
];
const light = new V(-3, 6, 1);
function nearest(origin, dir) {
    let best = Infinity, thing = null;
    for (const s of scene) {
        const t = s.hit(origin, dir);
        if (t > 0 && t < best) { best = t; thing = s; }
    }
    return thing === null ? null : { t: best, thing };
}
function trace(origin, dir, depth) {
    const found = nearest(origin, dir);
    if (found === null) return new V(0.1, 0.1, 0.2);
    const p = origin.add(dir.mul(found.t));
    const n = found.thing.normal(p);
    const toLight = light.sub(p).norm();
    const shadow = nearest(p, toLight);
    let diffuse = Math.max(0, n.dot(toLight));
    if (shadow !== null) diffuse *= 0.3;
    let color = found.thing.color.mul(0.1 + 0.9 * diffuse);
    if (depth < 2 && found.thing.shine > 0) {
        const reflected = dir.sub(n.mul(2 * dir.dot(n)));
        color = color.mul(1 - found.thing.shine).add(trace(p, reflected, depth + 1).mul(found.thing.shine));
    }
    return color;
}
const W = 200, H = 150;
const eye = new V(0, 1.5, 0);
let checksum = 0;
for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
        const dir = new V((x - W / 2) / H, (H / 2 - y) / H, 1).norm();
        const c = trace(eye, dir, 0);
        const r = Math.min(255, Math.floor(c.x * 255)), g = Math.min(255, Math.floor(c.y * 255)), b = Math.min(255, Math.floor(c.z * 255));
        checksum = (checksum * 31 + r * 65536 + g * 256 + b) % 1000000007;
    }
}
console.log('raytrace', checksum);
