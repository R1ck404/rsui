// A page with a lot of CSS in it, of the kinds that take the most working out: `calc()`, `clamp()`, `min()` and `max()`, sizes in
// `vw`, `vh` and `em`, gradients of every kind, several shadows, `!important`, variables, transitions, rules about what's inside
// something and `::before`. It's for measuring how long `rsui/web` takes with CSS.

import { useState } from 'rsui';
import { addCss, dom } from 'rsui/web';

addCss(`
  :root {
    --hue: 222;
    --ink: hsl(var(--hue), 30%, 18%);
    --paper: hsl(var(--hue), 40%, 98%);
    --brand: hsl(var(--hue), 85%, 56%);
    --brand-2: #8b5cf6;
    --space: clamp(8px, 1.5vw, 20px);
    --wide: min(1100px, 92vw);
    --round: 0.75rem;
  }
  html { font-size: 16px; }
  body {
    margin: 0;
    font: 15px/1.5 "Inter", "Segoe UI", sans-serif;
    color: var(--ink);
    background: radial-gradient(circle at 10% 0%, rgba(139, 92, 246, 0.18), transparent 40%),
      linear-gradient(180deg, var(--paper), #eef2ff);
  }
  .hero {
    padding: calc(var(--space) * 3) var(--space);
    text-align: center;
    background: linear-gradient(135deg, var(--brand) 0%, var(--brand-2) 100%);
    color: white;
    border-radius: 0 0 var(--round) var(--round) / 0 0 2rem 2rem;
  }
  .hero h1 { font-size: clamp(28px, 4vw, 56px); margin: 0 0 0.25em; letter-spacing: -0.02em; text-shadow: 0 2px 8px rgba(0, 0, 0, 0.25); }
  .hero p { font-size: 1.15em; opacity: 0.9; max-width: 40em; margin: 0 auto; }
  .grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: calc(var(--space) + 4px);
    width: var(--wide);
    margin: calc(var(--space) * -2) auto 0;
    padding: 0 var(--space);
  }
  .tile {
    position: relative;
    padding: 1.25em 1.5em;
    border-radius: var(--round);
    background: white;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05), 0 8px 24px -12px rgba(30, 41, 80, 0.35), inset 0 0 0 1px rgba(30, 41, 80, 0.06);
    transition: transform 0.2s ease-out, box-shadow 0.2s ease-out;
  }
  .tile:hover { background: #fbfcff; }
  .tile h3 { margin: 0 0 0.5em; font-size: 1.1rem; }
  .tile h3::before { content: "\\2022  "; color: var(--brand); }
  .tile .note { font-size: 0.85em; color: hsl(var(--hue), 12%, 45%); }
  .tile.featured {
    background: repeating-linear-gradient(45deg, #fff7ed, #fff7ed 10px, #ffedd5 10px, #ffedd5 20px);
    border: 2px dashed #fb923c !important;
  }
  .meter { height: max(6px, 0.5vh); border-radius: 99px; background: #e5e7eb; overflow: hidden; margin-top: 0.75em; }
  .meter > div { height: 100%; background: linear-gradient(to right, #22c55e, #eab308 60%, #ef4444); }
  .chips { display: flex; flex-wrap: wrap; gap: 0.5em; margin-top: 1em; }
  .chip { padding: 0.25em 0.75em; border-radius: 999px; border: 1px solid currentColor; font-size: 0.8em; color: var(--brand); }
  .chip.on { background: var(--brand); color: white !important; }
  .footer { width: var(--wide); margin: calc(var(--space) * 2) auto; padding: var(--space); font-size: 0.8rem; color: #64748b; border-top: 1px solid rgba(100, 116, 139, 0.3); }
  @media (max-width: 800px) { .grid { grid-template-columns: 1fr; } }
  @keyframes pulse {
    from { opacity: 0.6; transform: scale(0.98); }
    to { opacity: 1; transform: scale(1); }
  }
  .badge {
    position: absolute;
    top: calc(0.75em - 2px);
    right: max(0.75em, 8px);
    padding: 0.15em 0.6em;
    font-size: 0.7em;
    font-weight: 700;
    text-transform: uppercase;
    color: white;
    background: radial-gradient(ellipse at top left, #f472b6, #db2777);
    border-radius: 4px 12px / 4px 8px;
    animation: pulse 1.2s ease-in-out infinite alternate;
  }
  .big {
    font-size: 1.5em;
    font-weight: 800;
    line-height: 1.1;
    margin: 0.2em 0;
    background: linear-gradient(90deg, var(--brand), var(--brand-2));
    -webkit-background-clip: text;
    color: transparent;
  }
  .big.large { font-size: 2.25em; }
  .toggle {
    margin: 0 0.25em;
    padding: calc(0.5em + 1px) 1.25em;
    font: inherit;
    font-weight: 600;
    border: 0;
    border-radius: calc(var(--round) * 2);
    color: var(--ink);
    background: rgba(255, 255, 255, 0.85);
    box-shadow: 0 2px 0 rgba(0, 0, 0, 0.15);
    transition: background-color 150ms linear, color 150ms linear;
  }
  .toggle.on { color: white; background: var(--ink); }
  .toggle:active { transform: translateY(1px); }
`);

const TILES = Array.from({ length: 12 }, (_, at) => ({
  title: ['Throughput', 'Latency', 'Errors', 'Saturation', 'Queue depth', 'Cache hits'][at % 6] + ' ' + (at + 1),
  value: `${(at * 37) % 100}.${at % 10}`,
  unit: ['k/s', 'ms', '%', '%', 'jobs', '%'][at % 6],
  share: ((at * 29) % 90) + 10,
  featured: at % 5 === 0,
  tags: ['api', 'eu-west', 'v2', 'canary'].slice(0, 1 + (at % 4)),
}));

export default function App() {
  const [large, setLarge] = useState(false);
  const [picked, setPicked] = useState('api');
  return (
    <dom.div>
      <dom.section className="hero">
        <dom.h1>Service health</dom.h1>
        <dom.p>Everything that's running, how fast, and how much of it is going wrong, updated as it happens.</dom.p>
        <dom.div style={{ marginTop: 'calc(1em + 4px)' }}>
          <dom.button key="small" className={large ? 'toggle' : 'toggle on'} onClick={() => setLarge(false)}>
            Compact
          </dom.button>
          <dom.button key="large" className={large ? 'toggle on' : 'toggle'} onClick={() => setLarge(true)}>
            Large
          </dom.button>
        </dom.div>
      </dom.section>
      <dom.div className="grid">
        {TILES.map((tile) => (
          <dom.div key={tile.title} className={tile.featured ? 'tile featured' : 'tile'}>
            {tile.featured ? <dom.span className="badge">new</dom.span> : null}
            <dom.h3>{tile.title}</dom.h3>
            <dom.div className={large ? 'big large' : 'big'}>
              {tile.value}
              <dom.small style={{ fontSize: '0.5em', marginLeft: '0.25em', WebkitTextFillColor: 'initial' }}>{tile.unit}</dom.small>
            </dom.div>
            <dom.div className="note">{tile.share}% of capacity in the last 5 minutes</dom.div>
            <dom.div className="meter">
              <dom.div style={{ width: `calc(${tile.share}% - 2px)`, minWidth: '0.5em' }} />
            </dom.div>
            <dom.div className="chips">
              {tile.tags.map((tag) => (
                <dom.span key={tag} className={tag === picked ? 'chip on' : 'chip'} onClick={() => setPicked(tag)}>
                  {tag}
                </dom.span>
              ))}
            </dom.div>
          </dom.div>
        ))}
      </dom.div>
      <dom.div className="footer" style={{ textAlign: 'center', letterSpacing: '0.02em' }}>
        Sizes here come from calc(), clamp(), min() and max(), in px, em, rem, vw and vh.
      </dom.div>
    </dom.div>
  );
}
