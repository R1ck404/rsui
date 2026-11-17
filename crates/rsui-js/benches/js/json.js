// JSON.stringify and JSON.parse of a realistic payload: invoices with lines, nested objects, escapes and numbers.
function payload(count) {
    const invoices = [];
    for (let i = 0; i < count; i++) {
        const lines = [];
        for (let k = 0; k < 1 + (i % 6); k++) {
            lines.push({ id: k, what: `Work item ${k} for "client" ${i}\twith a tab`, quantity: String(k + 1), price: String((i * 7 + k) % 300 + 0.5) });
        }
        invoices.push({
            id: i,
            number: `INV-${1000 + i}`,
            client: { name: `Client ${i % 50}`, email: `client${i % 50}@example.com`, address: ['Street 1', '1234 AB', 'Town'] },
            issued: '2024-03-01',
            due: '2024-04-01',
            status: ['draft', 'sent', 'paid'][i % 3],
            taxRate: 21,
            paid: i % 3 === 2,
            notes: i % 5 === 0 ? null : 'Thanks for your business — ünïcödé ✓',
            lines,
        });
    }
    return { version: 3, nextId: count, settings: { currency: 'EUR', locale: 'nl-NL', tax: 21 }, invoices };
}
const data = payload(800);
let total = 0;
for (let r = 0; r < 24; r++) {
    const text = JSON.stringify(data);
    total += text.length;
    const back = JSON.parse(text);
    total += back.invoices.length + back.invoices[r].lines.length;
}
const pretty = JSON.stringify(data, null, 2);
total += pretty.length;
const again = JSON.parse(pretty, (key, value) => (key === 'price' ? Number(value) : value));
total += Math.round(again.invoices[3].lines[0].price * 2);
console.log('json', total);
