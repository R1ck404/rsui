// String methods: split, join, replace, replaceAll, indexOf, slice, case changes, trim, padStart, startsWith, includes.
const words = ['alpha', 'Beta', 'gamma', 'DELTA', 'epsilon', 'zeta', 'eta', 'theta', 'iota', 'kappa'];
const line = '  ' + words.join(', ') + ' and some more words at the end  ';
let total = 0;
for (let r = 0; r < 50000; r++) {
    const parts = line.trim().split(', ');
    total += parts.length;
    const up = parts[r % parts.length].toUpperCase();
    total += up.charCodeAt(0);
    const low = line.toLowerCase();
    total += low.indexOf('kappa') + low.lastIndexOf('a');
    total += line.replace('gamma', 'GAMMA!').length;
    total += line.replaceAll('a', 'aa').length;
    total += line.slice(5, 25).length + line.substring(10, 3).length;
    total += String(r).padStart(8, '0').length;
    if (line.includes('zeta')) total++;
    if (parts[0].startsWith('al')) total++;
    if (line.endsWith('end  ')) total++;
    total += `${parts[1]}-${r}`.length;
}
const csv = [];
for (let i = 0; i < 2000; i++) csv.push(`${i},name ${i},${i * 3},${i % 2 ? 'yes' : 'no'}`);
const text = csv.join('\n');
for (let r = 0; r < 30; r++) {
    for (const row of text.split('\n')) {
        const cells = row.split(',');
        total += cells[2].length + (cells[3] === 'yes' ? 1 : 0);
    }
}
console.log('string_ops', total);
