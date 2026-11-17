// Builtins apps lean on that the other benches don't: money formatting with toFixed and a regex, default sorts of
// strings and numbers, sorting what's already sorted, indexOf/includes, and numbers written as strings.
let total = 0;
const money = (value) => '€' + value.toFixed(2).replace(/\B(?=(\d{3})+(?!\d))/g, ',');
for (let i = 0; i < 60000; i++) total += money(i * 137.31).length;
let seed = 99;
function rand() { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed; }
const words = [];
for (let i = 0; i < 20000; i++) words.push('w' + (rand() % 50000).toString(36) + 'x');
const numbers = [];
for (let i = 0; i < 20000; i++) numbers.push(rand() % 100000);
for (let r = 0; r < 2; r++) {
    const sortedWords = words.slice().sort();
    total += sortedWords[0].length + sortedWords[10000].charCodeAt(1);
    const sortedNumbers = numbers.slice().sort();
    total += sortedNumbers[5000] % 1000;
    const again = sortedNumbers.slice().sort((a, b) => a - b);
    total += again.slice().sort((a, b) => a - b)[19999] % 1000;
}
const ids = [];
for (let i = 0; i < 500; i++) ids.push(i * 3);
const names = ids.map((id) => 'user-' + id);
for (let r = 0; r < 4000; r++) {
    total += ids.indexOf((r * 7) % 1500) + (ids.includes(r % 1501) ? 1 : 0);
    total += names.indexOf('user-' + ((r * 3) % 1500)) + (names.includes('nobody') ? 1 : 0);
}
for (let i = 0; i < 90000; i++) total += String(i * 1.25 + 0.5).length + (i / 7).toFixed(3).length;
console.log('builtin_misc', total);
