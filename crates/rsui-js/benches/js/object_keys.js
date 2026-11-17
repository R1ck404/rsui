// Objects used as dictionaries: computed keys, many keys, Object.keys/values/entries, for-in, `in`, delete, hasOwn.
let total = 0;
for (let r = 0; r < 32; r++) {
    const dict = {};
    for (let i = 0; i < 2000; i++) dict['k' + (i * 7 % 2000)] = i;
    for (let i = 0; i < 2000; i++) total += dict['k' + i];
    total += Object.keys(dict).length;
    for (const v of Object.values(dict)) total += v & 1;
    for (const [k, v] of Object.entries(dict)) total += k.length + (v & 3);
    for (const k in dict) if (k.length === 3) total++;
    for (let i = 0; i < 2000; i += 2) delete dict['k' + i];
    for (let i = 0; i < 2000; i++) if (('k' + i) in dict) total++;
    for (let i = 0; i < 2000; i++) if (Object.hasOwn(dict, 'k' + i)) total++;
    const counts = {};
    for (let i = 0; i < 20000; i++) {
        const key = 'w' + (i % 97);
        counts[key] = (counts[key] || 0) + 1;
    }
    total += counts.w5 + Object.keys(counts).length;
    const indexed = {};
    for (let i = 0; i < 5000; i++) indexed[i] = i * 2;
    for (let i = 0; i < 5000; i++) total += indexed[i];
    total = total % 1000000007;
}
console.log('object_keys', total);
