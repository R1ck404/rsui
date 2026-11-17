// Buffer: text to bytes and back in UTF-8, hex, base64 and Latin-1, joining, comparing and searching bytes.
const words = ['invoice', 'client', 'amount', 'ünïcödé', 'paid', '✓ done', 'total', 'tax'];
let seed = 7;
const next = () => ((seed = (Math.imul(seed, 1103515245) + 12345) | 0) >>> 16);
const lines = [];
for (let i = 0; i < 400; i++) {
    lines.push(`${i} ${words[next() % words.length]} ${words[next() % words.length]} ${next() % 1000}`);
}
let total = 0;
for (let r = 0; r < 20; r++) {
    const parts = lines.map((line) => Buffer.from(line));
    const all = Buffer.concat(parts);
    total += all.length;
    const hex = all.toString('hex');
    const base64 = all.toString('base64');
    total += hex.length + base64.length;
    const back = Buffer.from(base64, 'base64');
    total += back.equals(all) ? 1 : 0;
    total += Buffer.from(hex, 'hex').compare(all) + 1;
    total += back.toString('utf8').length + back.toString('latin1').length;
    for (let i = 0; i < 40; i++) {
        total += all.indexOf(words[i % words.length], i * 7) + all.lastIndexOf(0x20, all.length - i);
    }
    for (const part of parts) {
        total += part.toString().length + Buffer.byteLength(part.toString('hex'), 'hex');
    }
    total %= 1000000007;
}
console.log('buffer', total);
