// Sorting: numbers with a comparator, strings by default order, and objects by several keys (stable).
let seed = 12345;
function rand() { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed; }
let total = 0;
for (let r = 0; r < 3; r++) {
    const nums = [];
    for (let i = 0; i < 60000; i++) nums.push(rand() % 100000);
    nums.sort((a, b) => a - b);
    total += nums[0] + nums[30000] + nums[59999];
}
const words = [];
for (let i = 0; i < 30000; i++) words.push('w' + (rand() % 50000).toString(36) + 'x');
words.sort();
total += words[0].length + words[15000].charCodeAt(1) + words[29999].length;
const people = [];
for (let i = 0; i < 30000; i++) people.push({ name: 'p' + (rand() % 500), age: rand() % 90, id: i });
people.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : a.age - b.age));
let checksum = 0;
for (let i = 0; i < people.length; i += 97) checksum = (checksum * 31 + people[i].id) % 1000000007;
const sorted = people.slice().sort((a, b) => a.age - b.age).map((p) => p.id);
for (let i = 0; i < sorted.length; i += 101) checksum = (checksum * 31 + sorted[i]) % 1000000007;
console.log('array_sort', total, checksum);
