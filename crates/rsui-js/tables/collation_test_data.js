// Makes `tests/data/collation-icu.txt`, the order ICU puts names in with each set of options, for `collate.rs`'s tests.
// Run it with Node (whose ICU is Edge's) from `crates/rsui-js`: `node tables/collation_test_data.js`.
'use strict';
const fs = require('fs');

let seed = 2024;
const random = (n) => {
    seed = (Math.imul(seed, 1103515245) + 12345) & 0x7fffffff;
    return seed % n;
};
const pick = (list) => list[random(list.length)];

// Single characters from the scripts and symbols file names have, so every part of the table is looked at.
const single = [];
for (let point = 0x20; point < 0x7f; point += 1) single.push(String.fromCodePoint(point));
for (let point = 0xa0; point < 0x250; point += 1) single.push(String.fromCodePoint(point));
for (const [from, to] of [[0x370, 0x3ff], [0x400, 0x45f], [0x5d0, 0x5ea], [0x627, 0x64a], [0x905, 0x939], [0xe01, 0xe2e],
    [0x3041, 0x3096], [0x30a1, 0x30fa], [0xff01, 0xff5e], [0x2010, 0x2027], [0x20a0, 0x20c0], [0x2150, 0x2189],
    [0x1100, 0x1112], [0x2460, 0x2473]]) {
    for (let point = from; point <= to; point += 1) single.push(String.fromCodePoint(point));
}
for (let point = 0x4e00; point < 0x9fff; point += 97) single.push(String.fromCodePoint(point));
for (let point = 0x3400; point < 0x4dbf; point += 211) single.push(String.fromCodePoint(point));
for (let point = 0xac00; point < 0xd7a3; point += 331) single.push(String.fromCodePoint(point));
// Han after Extension A isn't here: rsui puts it after the rest by code point, where ICU has radical-stroke order.
single.push('\uF900', '\uFA0E', '\u3280', '\u2F00', '\u3400', '\u{1F600}', '\u{1F4C1}', '\u{1D400}', '\u{10400}', '\u{E000}', '\u{FFFD}');

const curated = [
    'résumé', 'resume', 'Resume', 'RESUME', 'résume', 'resumé', 'résumé', 'cote', 'côte', 'coté', 'côté',
    'a', 'A', 'á', 'Á', 'ä', 'å', 'æ', 'Æ', 'ae', 'AE', 'af', 'b', 'ß', 'ss', 'SS', 'st', 'œ', 'oe', 'ø', 'o', 'p', 'ı', 'i',
    'İ', 'I', 'ﬁ', 'fi', 'ǅ', 'dž', 'Ǆ', 'ĳ', 'ij', 'ŉ', 'Ａ', 'ａ', 'ｶ', 'カ', 'か', 'が', 'ゕ', 'ヵ', 'ア', 'あ', 'ぁ', 'ァ',
    'file1', 'file2', 'file10', 'file02', 'file002', 'file 1', 'file-1', 'file_1', 'file.1', 'file1a', 'file1b', 'file01a',
    'v1.2.10', 'v1.2.9', 'v1.10', 'x٣', 'x3', 'x３', 'x²', 'x2', '0', '00', '000', '9', '10', '99999999999999999999',
    '100000000000000000000', 'a0b', 'ab', 'a b', 'a-b', 'a_b', 'a.b', "a'b", 'a,b', 'a(b', 'a)b', 'a[b', 'a@b', 'a#b',
    'a$b', 'a%b', 'a&b', 'a*b', 'a+b', 'a=b', 'a~b', 'a`b', 'a^b', 'a|b', 'a\\b', 'a/b', 'a:b', 'a;b', 'a!b', 'a?b',
    'a"b', 'a<b', 'a>b', 'a{b', 'a}b', 'a·b', 'l·l', 'l·m', 'll', 'lm', 'L·L', 'Москва', 'москва',
    'Минск', 'Ελλάδα', 'ελλάδα', 'Αθήνα', 'שלום', 'مرحبا', 'नमस्ते', 'สวัสดี', 'เก', 'แก', 'กา', '東京', '東京都', '京都', '大阪', '北京', '上海', '台北', '香港', '漢字', '汉字', '文件', '文件夹', '新しいフォルダー', '写真', '資料', '会議', '会议',
    '日本', '日本語', '中文', '中国', '한국어', '한국', '서울', 'ひらがな', 'カタカナ', 'ﾊﾟｰﾄ', 'パート', 'ハート', 'ばあと',
    '\u{1F600} smile', '\u{1F4C1} folder', 'Ångström', 'Angstrom', 'Zürich', 'Zurich', 'zürich', 'über', 'Uber', 'uber',
    'año', 'ano', 'Año', 'naïve', 'naive', 'Ǿ', 'ǿ', 'ǿ', 'Đ', 'đ', 'd', 'Ł', 'ł', 'l', 'Þ', 'þ', 'th', 'z', 'Ž', 'ž',
    '', ' ', '  ', '\t', 'a\u0000b', 'a​b', 'Ä', 'Ä', 'ä́', 'á̈', 'ệ',
    'ệ', 'ệ', '́', 'ệb', 'ế', 'ệ', 'ệa', 'éa', 'Й', 'Й', 'й', 'й', 'ӥ', 'ạ́', 'Microsoft-Windows-Foo', 'microsoft.windows.foo', 'amd64_microsoft-windows-a_31bf',
    'amd64_microsoft-windows-a_31bf3856ad364e35_10.0.26100.1_none_0a', 'wow64_microsoft-windows-a_31bf', 'x86_net',
    'README.md', 'readme.md', 'ReadMe.md', 'Readme.MD', 'readme', 'readme.', 'readme-', 'readme_', 'readme~',
];

const words = ['report', 'Report', 'photo', 'IMG', 'img', 'Résumé', 'resume', 'café', 'Cafe', 'naïve', 'Ångström',
    'Zürich', 'über', 'Uber', 'año', 'Ελλάδα', 'Москва', 'москва', '東京', '日本語', '中文', '한국어', 'ひらがな',
    'カタカナ', 'notes', 'Notes', 'NOTES', 'draft', 'final', 'backup', 'a', 'B', 'c', 'Æsir', 'œuvre', 'straße',
    'strasse', 'émile', 'Emile', 'élan', 'ça', 'x86', 'amd64', 'wow64', 'microsoft-windows', 'Microsoft.Windows', 'setup',
    'readme', 'README', 'data', 'ﬁle', 'file', 'File', 'İstanbul', 'istanbul'];
const separators = ['', ' ', '-', '_', '.', ' (', '~', "'", ',', '+', '&', '#', '@', '!', '$', '%', '='];
const extensions = ['.txt', '.png', '.JPG', '.docx', '.tar.gz', '', '.manifest', '.dll', '.md'];
const generated = new Set();
while (generated.size < 700) {
    let name = pick(words);
    const parts = random(4);
    for (let i = 0; i < parts; i += 1) {
        const separator = pick(separators);
        const number = String(random(3) === 0 ? random(100000) : random(30)).padStart(random(3) === 0 ? 3 : 1, '0');
        name += separator + (random(3) === 0 ? number : pick(words));
        if (separator === ' (') name += ')';
    }
    generated.add(name + pick(extensions));
}

const names = [...new Set([...single, ...curated, ...generated])];
for (const name of names) {
    if (/[\n\r]/.test(name)) throw new Error('a name has a line break');
}
const sets = [
    {},
    { sensitivity: 'base' },
    { sensitivity: 'accent' },
    { sensitivity: 'case' },
    { numeric: true },
    { numeric: true, sensitivity: 'base' },
    { caseFirst: 'upper' },
    { ignorePunctuation: true },
];
const lines = [
    "# Made by `tables/collation_test_data.js` with Node's ICU " + process.versions.icu + '. Each order is the names, by',
    '# number, in the order ICU puts them in with those options, each with how it compares to the one before.',
    'names ' + names.length,
    ...names.map((name) => name.replace(/\t/g, '\\t').replace(/\0/g, '\\0')),
];
for (const options of sets) {
    const collator = new Intl.Collator('en', options);
    const order = names.map((_, index) => index).sort((a, b) => collator.compare(names[a], names[b]) || a - b);
    const words = Object.entries(options).map(([key, value]) => (value === true ? key : `${key}=${value}`));
    lines.push('order ' + (words.join(' ') || 'default'));
    lines.push(order.map((index, at) => (at === 0 ? `${index}` : `${index}:${collator.compare(names[order[at - 1]], names[index])}`)).join(' '));
}
fs.writeFileSync('tests/data/collation-icu.txt', lines.join('\n') + '\n');
console.log(names.length, 'names');
