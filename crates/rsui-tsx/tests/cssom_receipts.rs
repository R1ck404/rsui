//! Reusing an owned custom-property overlay must preserve ordinary CSSOM reads.

mod hands;

#[test]
fn custom_property_overlay_is_reused_without_losing_transform_or_removal_updates() {
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-cssom-receipts-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(folder.join("app.css"), "body { margin: 0; }\n").unwrap();
    std::fs::write(
        &file,
        r#"
        import './app.css';
        import { PageWindow, dom } from 'rsui/web';
        import { useState } from 'react';
        import * as rsui from 'rsui';
        export default function App() {
            const [said, setSaid] = useState('waiting');
            return <PageWindow>
                <dom.button key="write-cssom" onClick={() => {
                    const target = document.getElementById('target');
                    target.style.setProperty('--measure-width', '80px');
                    target.style.setProperty('--shift', '20px');
                    setTimeout(() => {
                        const native = rsui.__host().css;
                        const original = native.cssInline;
                        let reads = 0, repeated = [];
                        native.cssInline = (...args) => { reads++; return original(...args); };
                        try {
                            for (let at = 0; at < 3; at++) {
                                const r = target.getBoundingClientRect();
                                repeated.push([Math.round(r.left), Math.round(r.top), Math.round(r.width),
                                    getComputedStyle(target).position].join(','));
                            }
                        } finally {
                            native.cssInline = original;
                        }
                        const bounded = reads <= 1;
                        // This ordinary property must still be read before its
                        // native render, including the raw percentage transform.
                        target.style.transform = 'translateX(50%)';
                        const pending = Math.round(target.getBoundingClientRect().left);
                        target.style.removeProperty('transform');
                        setTimeout(() => {
                            const removed = target.getBoundingClientRect();
                            setSaid(repeated.join('|') + '|bounded ' + bounded + '|pending ' + pending
                                + '|removed ' + Math.round(removed.left) + ',' + Math.round(removed.width)
                                + ',' + getComputedStyle(target).position);
                        }, 0);
                    }, 0);
                }}>write CSSOM</dom.button>
                <dom.div id="target" style={{position: 'fixed', left: 20, top: 50, height: 20,
                    width: 'var(--measure-width, 50px)', '--measure-width': '50px', '--shift': '0px',
                    transform: 'translateX(var(--shift, 0px))'}}>target</dom.div>
                <dom.p style={{position: 'fixed', left: 0, top: 200}}>{said}</dom.p>
            </PageWindow>;
        }
        "#,
    )
    .unwrap();
    let mut ui = hands::Hands::open(&file, (500, 350));
    ui.click("write-cssom");
    ui.wait_for(
        "40,50,80,fixed|40,50,80,fixed|40,50,80,fixed|bounded true|pending 60|removed 20,80,fixed",
    );
}
