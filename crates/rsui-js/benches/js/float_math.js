// Floating-point math: an n-body simulation (only + - * / and sqrt, so every engine agrees exactly), and a loop of
// Math functions rounded so that last-bit differences between engines don't show.
const PI = Math.PI, SOLAR_MASS = 4 * PI * PI, DAYS_PER_YEAR = 365.24;
function Body(x, y, z, vx, vy, vz, mass) {
    this.x = x; this.y = y; this.z = z; this.vx = vx; this.vy = vy; this.vz = vz; this.mass = mass;
}
function bodies() {
    return [
        new Body(0, 0, 0, 0, 0, 0, SOLAR_MASS),
        new Body(4.84143144246472090e+00, -1.16032004402742839e+00, -1.03622044471123109e-01,
            1.66007664274403694e-03 * DAYS_PER_YEAR, 7.69901118419740425e-03 * DAYS_PER_YEAR,
            -6.90460016972063023e-05 * DAYS_PER_YEAR, 9.54791938424326609e-04 * SOLAR_MASS),
        new Body(8.34336671824457987e+00, 4.12479856412430479e+00, -4.03523417114321381e-01,
            -2.76742510726862411e-03 * DAYS_PER_YEAR, 4.99852801234917238e-03 * DAYS_PER_YEAR,
            2.30417297573763929e-05 * DAYS_PER_YEAR, 2.85885980666130812e-04 * SOLAR_MASS),
        new Body(1.28943695621391310e+01, -1.51111514016986312e+01, -2.23307578892655734e-01,
            2.96460137564761618e-03 * DAYS_PER_YEAR, 2.37847173959480950e-03 * DAYS_PER_YEAR,
            -2.96589568540237556e-05 * DAYS_PER_YEAR, 4.36624404335156298e-05 * SOLAR_MASS),
        new Body(1.53796971148509165e+01, -2.59193146099879641e+01, 1.79258772950371181e-01,
            2.68067772490389322e-03 * DAYS_PER_YEAR, 1.62824170038242295e-03 * DAYS_PER_YEAR,
            -9.51592254519715870e-05 * DAYS_PER_YEAR, 5.15138902046611451e-05 * SOLAR_MASS),
    ];
}
function offset(list) {
    let px = 0, py = 0, pz = 0;
    for (const b of list) { px += b.vx * b.mass; py += b.vy * b.mass; pz += b.vz * b.mass; }
    list[0].vx = -px / SOLAR_MASS; list[0].vy = -py / SOLAR_MASS; list[0].vz = -pz / SOLAR_MASS;
}
function advance(list, dt) {
    const n = list.length;
    for (let i = 0; i < n; i++) {
        const a = list[i];
        for (let j = i + 1; j < n; j++) {
            const b = list[j];
            const dx = a.x - b.x, dy = a.y - b.y, dz = a.z - b.z;
            const d2 = dx * dx + dy * dy + dz * dz;
            const mag = dt / (d2 * Math.sqrt(d2));
            a.vx -= dx * b.mass * mag; a.vy -= dy * b.mass * mag; a.vz -= dz * b.mass * mag;
            b.vx += dx * a.mass * mag; b.vy += dy * a.mass * mag; b.vz += dz * a.mass * mag;
        }
    }
    for (let i = 0; i < n; i++) {
        const b = list[i];
        b.x += dt * b.vx; b.y += dt * b.vy; b.z += dt * b.vz;
    }
}
function energy(list) {
    let e = 0;
    for (let i = 0; i < list.length; i++) {
        const a = list[i];
        e += 0.5 * a.mass * (a.vx * a.vx + a.vy * a.vy + a.vz * a.vz);
        for (let j = i + 1; j < list.length; j++) {
            const b = list[j];
            const dx = a.x - b.x, dy = a.y - b.y, dz = a.z - b.z;
            e -= (a.mass * b.mass) / Math.sqrt(dx * dx + dy * dy + dz * dz);
        }
    }
    return e;
}
const system = bodies();
offset(system);
const before = energy(system);
for (let i = 0; i < 120000; i++) advance(system, 0.01);
const after = energy(system);
let wave = 0;
for (let i = 0; i < 300000; i++) {
    const x = i * 0.001;
    wave += Math.sin(x) * Math.cos(x * 0.5) + Math.sqrt(x) - Math.floor(x) + Math.abs(Math.exp(-x) - 0.5) + Math.atan2(x, 1 + x);
}
console.log('float_math', before.toFixed(9), after.toFixed(9), wave.toFixed(3));
