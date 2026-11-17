//! The experimental variable construction must preserve local ownership and mutable style inputs.
use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

#[test]
fn variable_deltas_match_full_construction_with_local_alias_mutation_and_noninheritance() {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-variable-deltas-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, r#"
        import { PageWindow } from 'rsui/web';
        import { useState } from 'react';
        const mutable = { '--mutable': '21px', width: 'var(--mutable)', height: 10 };
        export default function App() {
            const [phase, setPhase] = useState(0);
            globalThis.changeVariables = () => { mutable['--mutable'] = '44px'; setPhase(1); };
            return <PageWindow>
                <style>{`
                    @property --private { syntax: '<length>'; inherits: false; initial-value: 7px; }
                    * { --reset: 0; }
                    .measure { height: 10px; }
                `}</style>
                <div style={{'--shared': phase ? '33px' : '12px'}}>
                    <div style={{'--shared': '12px'}}>
                        <div id="shield" className="measure" style={{width: 'var(--shared)'}} />
                    </div>
                    <div style={{'--alias': 'var(--shared)'}}>
                        <div id="alias" className="measure" style={{width: 'var(--alias)'}} />
                    </div>
                    {/* An independent primitive change forces this host to render with the same, mutated style object. */}
                    <div id="mutable" data-tick={phase} className="measure" style={mutable} />
                    <div id="private-parent" className="measure" style={{'--private': '71px', width: 'var(--private)'}}>
                        <div id="private-child" className="measure" style={{width: 'var(--private)'}} />
                        <div id="private-own" className="measure" style={{'--private': '17px', width: 'var(--private)'}}>
                            <div id="private-grandchild" className="measure" style={{width: 'var(--private)'}} />
                        </div>
                    </div>
                </div>
            </PageWindow>;
        }
    "#).unwrap();
    let probe = "eval \"JSON.stringify(['shield','alias','mutable','private-parent','private-child','private-own','private-grandchild'].map(id => Math.round(document.getElementById(id).getBoundingClientRect().width)))\"";
    let steps = format!("wait 100; {probe}; eval \"changeVariables(); 'changed'\"; wait 100; {probe}");
    let mut variants = Vec::new();
    for enabled in ["0", "1"] {
        let sandbox = folder.join(format!("sandbox-{enabled}"));
        let output = Command::new(TOOL)
            .args(["--do", &steps]).arg(&file)
            .env("RSUI_VAR_DELTAS", enabled)
            .env("RSUI_SANDBOX", &sandbox)
            .env("APPDATA", &sandbox).env("LOCALAPPDATA", &sandbox)
            .env("USERPROFILE", &sandbox).env("HOME", &sandbox)
            .env("RSUI_STORAGE_FILE", sandbox.join("storage.json"))
            .env("RSUI_OFFLINE", "1").env("RSUI_RESTYLE_CHECK", "1")
            .output().unwrap();
        let said = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "variant {enabled}: {said}");
        assert!(!said.contains("TypeError") && !said.contains("MISSED"), "variant {enabled}: {said}");
        assert!(said.contains("[12,12,21,71,7,17,7]"), "variant {enabled}: {said}");
        assert!(said.contains("[12,33,44,71,7,17,7]"), "variant {enabled}: {said}");
        variants.push(said.lines().filter(|line| line.trim_start().starts_with("= [")).map(str::to_owned).collect::<Vec<_>>());
    }
    assert_eq!(variants[0], variants[1]);
}
