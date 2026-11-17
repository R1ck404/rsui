// Building strings: += in a loop, join, numbers to strings, template literals, and reading characters back.
let total = 0;
for (let r = 0; r < 10; r++) {
    let s = '';
    for (let i = 0; i < 20000; i++) s += 'item ' + i + ', ';
    total += s.length;
    let h = 0;
    for (let i = 0; i < s.length; i += 7) h = (h * 31 + s.charCodeAt(i)) | 0;
    total += h & 0xffff;
}
for (let r = 0; r < 10; r++) {
    const parts = [];
    for (let i = 0; i < 20000; i++) parts.push(`<li class="row ${i % 2 ? 'odd' : 'even'}">${i * 1.5}</li>`);
    const html = parts.join('\n');
    total += html.length;
}
for (let i = 0; i < 100000; i++) {
    const label = String(i) + ':' + (i / 8).toString() + '|' + (i % 255).toString(16);
    total += label.length;
}
console.log('string_build', total);
