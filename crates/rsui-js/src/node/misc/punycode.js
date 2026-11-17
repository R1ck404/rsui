'use strict';
// Node's `punycode`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = () => {
    const [BASE, TMIN, TMAX, SKEW, DAMP, FIRST_BIAS, FIRST_N] = [36, 1, 26, 38, 700, 72, 128];
    const adapt = (delta, count, first) => {
        let change = first ? Math.floor(delta / DAMP) : delta >> 1;
        change += Math.floor(change / count);
        let k = 0;
        while (change > ((BASE - TMIN) * TMAX) >> 1) {
            change = Math.floor(change / (BASE - TMIN));
            k += BASE;
        }
        return k + Math.floor(((BASE - TMIN + 1) * change) / (change + SKEW));
    };
    const digit = (value) => String.fromCharCode(value + 22 + 75 * (value < 26));
    const encode = (text) => {
        const points = [...text].map((character) => character.codePointAt(0));
        let out = points.filter((point) => point < FIRST_N).map((point) => String.fromCodePoint(point)).join('');
        const basic = out.length;
        let handled = basic;
        if (basic > 0) {
            out += '-';
        }
        let [n, delta, bias] = [FIRST_N, 0, FIRST_BIAS];
        while (handled < points.length) {
            const next = Math.min(...points.filter((point) => point >= n));
            delta += (next - n) * (handled + 1);
            n = next;
            for (const point of points) {
                if (point < n) {
                    delta += 1;
                }
                if (point === n) {
                    let q = delta;
                    for (let k = BASE; ; k += BASE) {
                        const t = k <= bias ? TMIN : k >= bias + TMAX ? TMAX : k - bias;
                        if (q < t) {
                            break;
                        }
                        out += digit(t + ((q - t) % (BASE - t)));
                        q = Math.floor((q - t) / (BASE - t));
                    }
                    out += digit(q);
                    bias = adapt(delta, handled + 1, handled === basic);
                    delta = 0;
                    handled += 1;
                }
            }
            delta += 1;
            n += 1;
        }
        return out;
    };
    const decode = (text) => {
        const split = text.lastIndexOf('-');
        const out = split > 0 ? [...text.slice(0, split)].map((character) => character.codePointAt(0)) : [];
        let [n, i, bias] = [FIRST_N, 0, FIRST_BIAS];
        for (let at = split > 0 ? split + 1 : 0; at < text.length;) {
            const old = i;
            let w = 1;
            for (let k = BASE; ; k += BASE) {
                const code = text.charCodeAt(at);
                at += 1;
                const value = code - 48 < 10 ? code - 22 : code - 65 < 26 ? code - 65 : code - 97 < 26 ? code - 97 : BASE;
                i += value * w;
                const t = k <= bias ? TMIN : k >= bias + TMAX ? TMAX : k - bias;
                if (value < t) {
                    break;
                }
                w *= BASE - t;
            }
            bias = adapt(i - old, out.length + 1, old === 0);
            n += Math.floor(i / (out.length + 1));
            i %= out.length + 1;
            out.splice(i, 0, n);
            i += 1;
        }
        return String.fromCodePoint(...out);
    };
    const eachLabel = (domain, change) => domain.split('.').map(change).join('.');
    return {
        encode,
        decode,
        toASCII: (domain) => eachLabel(domain, (label) => (/[^\0-\x7F]/.test(label) ? 'xn--' + encode(label) : label)),
        toUnicode: (domain) => eachLabel(domain, (label) => (label.startsWith('xn--') ? decode(label.slice(4).toLowerCase()) : label)),
        ucs2: { decode: (text) => [...text].map((character) => character.codePointAt(0)), encode: (points) => String.fromCodePoint(...points) },
        version: '2.1.0',
    };
};
