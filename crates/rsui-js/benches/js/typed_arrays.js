// Typed arrays: a box blur over an RGBA image in a Uint8ClampedArray, Float64Array math, Int32Array, set and subarray.
const W = 320, H = 200;
const image = new Uint8ClampedArray(W * H * 4);
for (let i = 0; i < image.length; i++) image[i] = (i * 2654435761) >>> 24;
const out = new Uint8ClampedArray(image.length);
let total = 0;
for (let pass = 0; pass < 6; pass++) {
    for (let y = 1; y < H - 1; y++) {
        for (let x = 1; x < W - 1; x++) {
            for (let c = 0; c < 4; c++) {
                const at = (y * W + x) * 4 + c;
                out[at] = (image[at] * 4 + image[at - 4] + image[at + 4] + image[at - W * 4] + image[at + W * 4]) >> 3;
            }
        }
    }
    image.set(out);
}
for (let i = 0; i < image.length; i += 13) total += image[i];
const f = new Float64Array(100000);
for (let i = 0; i < f.length; i++) f[i] = i * 0.5;
let s = 0;
for (let r = 0; r < 10; r++) for (let i = 0; i < f.length; i++) s += f[i] * 2 - 1;
total += Math.round(s);
const ints = new Int32Array(100000);
for (let i = 0; i < ints.length; i++) ints[i] = i ^ (i << 3);
const half = ints.subarray(0, 50000);
let x = 0;
for (let r = 0; r < 10; r++) for (let i = 0; i < half.length; i++) x = (x + half[i]) | 0;
total += x & 0xffff;
console.log('typed_arrays', total);
