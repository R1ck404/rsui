// Regular expressions: test, exec with groups in a loop, global replace with a callback, split, and matchAll.
const lines = [];
for (let i = 0; i < 3000; i++) {
    lines.push(`2024-${String(1 + (i % 12)).padStart(2, '0')}-${String(1 + (i % 28)).padStart(2, '0')} user${i}@example${i % 7}.com ` +
        `GET /api/items/${i}?page=${i % 10} 200 ${(i * 13) % 5000}ms "Mozilla/5.0 (Windows NT 10.0)"`);
}
const text = lines.join('\n');
const email = /([a-z0-9._-]+)@([a-z0-9-]+)\.(com|org)/;
const date = /(\d{4})-(\d{2})-(\d{2})/g;
let total = 0;
for (let r = 0; r < 4; r++) {
    for (const line of lines) {
        if (email.test(line)) total++;
        const m = email.exec(line);
        if (m) total += m[1].length + m[2].length;
    }
    date.lastIndex = 0;
    let m;
    while ((m = date.exec(text)) !== null) total += +m[2] + +m[3];
    total += text.replace(/\d+ms/g, (s) => '[' + s.length + ']').length;
    total += text.split(/\s+/).length;
    for (const hit of text.matchAll(/page=(\d)/g)) total += +hit[1];
    total += text.replace(/GET|POST/g, 'VERB').length;
    total += /Mozilla\/(\d)\.(\d)/.exec(lines[r])[1].length;
}
console.log('regex', total);
