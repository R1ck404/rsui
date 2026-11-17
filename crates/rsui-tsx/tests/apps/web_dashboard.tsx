// A dashboard written for the web, as a React app is: page CSS, classes, `style` objects and the page's own elements, all
// through `rsui/web`. It's for measuring how long `rsui/web` takes to load and to work CSS out,
// and it runs as an app too.

import { useState } from 'rsui';
import { addCss, dom } from 'rsui/web';

addCss(`
:root {
  --bg: #f4f6fb;
  --panel: #ffffff;
  --text: #1d2433;
  --muted: #6b7385;
  --accent: #3b6cf6;
  --accent-soft: rgba(59, 108, 246, 0.12);
  --good: #1f9d6b;
  --bad: #d64545;
  --radius: 10px;
  --gap: 16px;
}
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
body {
  font-family: "Segoe UI", system-ui, sans-serif;
  font-size: 14px;
  color: var(--text);
  background: var(--bg);
  line-height: 1.4;
}
.layout { display: flex; min-height: 100vh; }
.content { flex: 1; padding: 24px 32px; display: flex; flex-direction: column; gap: var(--gap); }
.row { display: flex; gap: var(--gap); flex-wrap: wrap; }
.card {
  background: var(--panel);
  border-radius: var(--radius);
  border: 1px solid #e3e7ef;
  box-shadow: 0 1px 2px rgba(16, 24, 40, 0.06), 0 1px 3px rgba(16, 24, 40, 0.1);
  padding: 16px 20px;
}
.card h2 { font-size: 15px; margin: 0 0 12px; font-weight: 600; }
.stat { flex: 1 1 180px; display: flex; flex-direction: column; gap: 4px; }
.stat .label { color: var(--muted); font-size: 12px; text-transform: uppercase; letter-spacing: 0.04em; }
.stat .value { font-size: 26px; font-weight: 700; }
.stat .change { font-size: 12px; }
.up { color: var(--good); }
.down { color: var(--bad); }
.badge { display: inline-block; padding: 2px 8px; border-radius: 999px; font-size: 11px; font-weight: 600; }
.badge.paid { background: rgba(31, 157, 107, 0.12); color: var(--good); }
.badge.late { background: rgba(214, 69, 69, 0.12); color: var(--bad); }
.badge.open { background: var(--accent-soft); color: var(--accent); }
table { width: 100%; border-collapse: collapse; }
th { text-align: left; color: var(--muted); font-weight: 500; font-size: 12px; padding: 8px 10px; border-bottom: 1px solid #e3e7ef; }
td { padding: 9px 10px; border-bottom: 1px solid #eef1f6; }
tr.selected td { background: var(--accent-soft); }
.muted { color: var(--muted); }
.bar-track { height: 8px; background: #eef1f6; border-radius: 4px; overflow: hidden; }
.bar-fill { height: 100%; background: var(--accent); border-radius: 4px; }
@media (max-width: 700px) { .content { padding: 12px; } }
.sidebar {
  width: 220px;
  background: #151a2d;
  color: #c9cfdd;
  padding: 20px 12px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.brand { font-size: 18px; font-weight: 700; color: #ffffff; padding: 4px 12px 18px; }
.nav-item { display: block; padding: 8px 12px; border-radius: 6px; color: #c9cfdd; background: transparent; cursor: pointer; }
.nav-item.active { color: #ffffff; background: rgba(255, 255, 255, 0.08); }
.nav-item:hover { background: rgba(255, 255, 255, 0.05); }
.header { display: flex; align-items: center; justify-content: space-between; }
.header h1 { font-size: 22px; margin: 0; }
.button {
  padding: 8px 14px;
  border-radius: 6px;
  border: 1px solid #d5dae4;
  font-weight: 600;
  cursor: pointer;
  background: white;
  color: var(--text);
}
.button.primary { background: var(--accent); color: white; border-color: transparent; }
.search { padding: 7px 10px; border: 1px solid #d5dae4; border-radius: 6px; width: 240px; }
`);

const STATS = [
  { label: 'Revenue', value: '$48,210', change: '+12.4%', up: true },
  { label: 'Invoices', value: '1,284', change: '+3.1%', up: true },
  { label: 'Overdue', value: '37', change: '-8.0%', up: false },
  { label: 'Customers', value: '412', change: '+1.2%', up: true },
];

const PAGES = ['Overview', 'Invoices', 'Customers', 'Reports', 'Settings'];

const STATUSES = ['paid', 'late', 'open'];
const ROWS = Array.from({ length: 40 }, (_, at) => ({
  id: 1000 + at,
  customer: ['Acme Corp', 'Globex', 'Initech', 'Umbrella', 'Hooli', 'Stark Industries'][at % 6],
  amount: ((at * 7919) % 5000) + 120,
  status: STATUSES[at % 3],
  due: `2026-${String((at % 12) + 1).padStart(2, '0')}-${String((at % 27) + 1).padStart(2, '0')}`,
}));

function Stat({ stat }) {
  return (
    <dom.div className="card stat">
      <dom.span className="label">{stat.label}</dom.span>
      <dom.span className="value">{stat.value}</dom.span>
      <dom.span className={'change ' + (stat.up ? 'up' : 'down')}>{stat.change} from last month</dom.span>
    </dom.div>
  );
}

function Bars() {
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun'];
  return (
    <dom.div className="card" style={{ flex: 1, minWidth: 260 }}>
      <dom.h2>Paid by month</dom.h2>
      {months.map((month, at) => (
        <dom.div key={month} style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 8 }}>
          <dom.span className="muted" style={{ width: 32 }}>{month}</dom.span>
          <dom.div className="bar-track" style={{ flex: 1 }}>
            <dom.div className="bar-fill" style={{ width: `${30 + ((at * 37) % 70)}%` }} />
          </dom.div>
        </dom.div>
      ))}
    </dom.div>
  );
}

export default function App() {
  const [page, setPage] = useState('Overview');
  const [selected, setSelected] = useState(1003);
  const [query, setQuery] = useState('');
  const shown = ROWS.filter((row) => row.customer.toLowerCase().includes(query.toLowerCase()));
  return (
    <dom.div className="layout">
      <dom.nav className="sidebar">
        <dom.div className="brand">Ledgerly</dom.div>
        {PAGES.map((name) => (
          <dom.a key={name} className={name === page ? 'nav-item active' : 'nav-item'} onClick={() => setPage(name)}>
            {name}
          </dom.a>
        ))}
      </dom.nav>
      <dom.main className="content">
        <dom.header className="header">
          <dom.h1>{page}</dom.h1>
          <dom.div className="row" style={{ alignItems: 'center', gap: 8 }}>
            <dom.input key="search" className="search" placeholder="Search customers" value={query} onChange={(e) => setQuery(e.target.value)} />
            <dom.button key="export" className="button">Export</dom.button>
            <dom.button key="new" className="button primary">
              New invoice
            </dom.button>
          </dom.div>
        </dom.header>
        <dom.div className="row">
          {STATS.map((stat) => (
            <Stat key={stat.label} stat={stat} />
          ))}
        </dom.div>
        <dom.div className="row">
          <Bars />
          <dom.div className="card" style={{ flex: 2, minWidth: 360 }}>
            <dom.h2>Recent invoices</dom.h2>
            <dom.table>
              <dom.thead>
                <dom.tr>
                  <dom.th>Number</dom.th>
                  <dom.th>Customer</dom.th>
                  <dom.th>Due</dom.th>
                  <dom.th style={{ textAlign: 'right' }}>Amount</dom.th>
                  <dom.th>Status</dom.th>
                </dom.tr>
              </dom.thead>
              <dom.tbody>
                {shown.map((row) => (
                  <dom.tr key={row.id} className={row.id === selected ? 'selected' : undefined} onClick={() => setSelected(row.id)}>
                    <dom.td>#{row.id}</dom.td>
                    <dom.td>{row.customer}</dom.td>
                    <dom.td className="muted">{row.due}</dom.td>
                    <dom.td style={{ textAlign: 'right', fontVariantNumeric: 'tabular-nums' }}>${row.amount.toLocaleString()}</dom.td>
                    <dom.td>
                      <dom.span className={'badge ' + row.status}>{row.status}</dom.span>
                    </dom.td>
                  </dom.tr>
                ))}
              </dom.tbody>
            </dom.table>
          </dom.div>
        </dom.div>
      </dom.main>
    </dom.div>
  );
}
