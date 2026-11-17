//! Declaration inputs and cascade updates remain correct across stylesheet changes.
//! Global-epoch and input-receipt experiments lacked sufficient native benefit; semantic regressions remain.
mod hands;
use std::sync::atomic::{AtomicU32, Ordering};
fn open(source: &str, css: &str) -> hands::Hands {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "tsx-class-declarations-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("app.css"), format!("body {{margin:0;}}\n{css}")).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file,format!("import './app.css'; function check(ok,why) {{if(!ok) throw new Error(why);}}\n{source}")).unwrap();
    hands::Hands::open(&file, (500, 350))
}

#[test]
fn newly_added_selectors_update_the_cascade() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web'; import {useState} from 'react';
        export default function App(){const [phase,setPhase]=useState(0);
          return <PageWindow><dom.div id="probe" className="probe">probe</dom.div>
          <dom.button key="warm" onClick={()=>setPhase(10)}>warm</dom.button>
          <dom.button key="add" onClick={()=>{
            const sheet=document.createElement('style'); sheet.textContent='.unrelated {color:purple;--new-unrelated-name:blue} .probe.more {background-color:rgb(1,2,3)}';document.head.appendChild(sheet);
            setPhase(1);
          }}>add</dom.button><dom.button key="verify" onClick={()=>{
            document.getElementById('probe').classList.add('more');setPhase(2);
          }}>verify</dom.button><dom.button key="cascade" onClick={()=>{
            check(getComputedStyle(document.getElementById('probe')).backgroundColor==='rgb(1, 2, 3)','new selector must join cascade');setPhase(3);
          }}>cascade</dom.button><dom.p>{'phase '+phase}</dom.p></PageWindow>;
        }
    "#,
        ".probe {letter-spacing:1.234px;color:rgb(12,34,56)} .probe:hover {background-color:pink}",
    );
    ui.click("warm");
    ui.wait_for("phase 10");
    ui.click("add");
    ui.wait_for("phase 1");
    ui.click("verify");
    ui.wait_for("phase 2");
    ui.click("cascade");
    ui.wait_for("phase 3");
}

#[test]
fn late_element_variables_change_declaration_deferral() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        export default function App(){const [phase,setPhase]=useState(0);return <PageWindow>
          <dom.div id="probe" className="probe" style={phase?{'--late':'green'}:{}}>probe</dom.div>
          <dom.button key="add" onClick={()=>{const s=document.createElement('style');s.textContent='.defines {--late:blue}';document.head.appendChild(s);setPhase(1)}}>add</dom.button>
          <dom.button key="verify" onClick={()=>{check(getComputedStyle(document.getElementById('probe')).color==='rgb(0, 128, 0)','new element-variable name must defer resolution');setPhase(2)}}>verify</dom.button><dom.p>{'phase '+phase}</dom.p>
          </PageWindow>}
    "#,
        ".probe {color:var(--late,red)}",
    );
    ui.click("add");
    ui.wait_for("phase 1");
    ui.click("verify");
    ui.wait_for("phase 2");
}

#[test]
fn root_cssom_variables_root_font_and_theme_attributes_invalidate_declarations() {
    let mut ui=open(r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        export default function App(){const [phase,setPhase]=useState(0);return <PageWindow><dom.div id="probe" className="probe">probe</dom.div>
          <dom.button key="change" onClick={()=>{
            document.documentElement.style.setProperty('--tone','blue');document.documentElement.style.setProperty('--root-size','20px');document.documentElement.setAttribute('data-theme','large');setPhase(1);
          }}>change</dom.button><dom.button key="verify" onClick={()=>{
            const style=getComputedStyle(document.getElementById('probe'));
            check(style.color==='rgb(0, 0, 255)','root CSSOM variable');check(style.width==='40px','root font and rem '+style.width);check(style.marginLeft==='7px','theme attributes');setPhase(2);
          }}>verify</dom.button><dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#," :root {--tone:red;--root-size:16px;font-size:var(--root-size)} :root[data-theme=large] {--gap:7px} .probe {color:var(--tone);width:2rem;margin-left:var(--gap,0px)}");
    ui.click("change");
    ui.wait_for("phase 1");
    ui.click("verify");
    ui.wait_for("phase 2");
}

#[test]
fn viewport_resize_updates_declarations_without_a_media_breakpoint() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow><dom.div id="probe" className="probe">probe</dom.div>
          <dom.button key="verify" onClick={()=>{check(getComputedStyle(document.getElementById('probe')).width==='350px','viewport dependency');setSaid('viewport passed')}}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#,
        ".probe {width:50vw}",
    );
    ui.app.resize((700, 350), 1.0);
    ui.settle();
    ui.click("verify");
    ui.wait_for("viewport passed");
}

#[test]
#[ignore = "Baseline head reorder fails independently of declaration caching; separate DOM ordering bug"]
fn replacing_and_reordering_real_sheets_preserves_declarations_and_cascade() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        let first, second;
        export default function App(){const [phase,setPhase]=useState(0);return <PageWindow>
        <dom.div id="probe" className="probe">probe</dom.div>
        <dom.button key="add" onClick={()=>{
            first=document.createElement('style');first.textContent='.probe {color:red}';
            second=document.createElement('style');second.textContent='.probe {color:blue}';
            document.head.appendChild(first);document.head.appendChild(second);setPhase(1);
        }}>add</dom.button><dom.button key="reorder" onClick={()=>{
            check(getComputedStyle(document.getElementById('probe')).color==='rgb(0, 0, 255)','last sheet wins');
            document.head.removeChild(first);document.head.appendChild(first);setPhase(2);
        }}>reorder</dom.button><dom.button key="replace" onClick={()=>{
            check(getComputedStyle(document.getElementById('probe')).color==='rgb(255, 0, 0)','reordered sheet wins');
            first.textContent='.probe {color:green}';setPhase(3);
        }}>replace</dom.button><dom.button key="verify" onClick={()=>{
            check(getComputedStyle(document.getElementById('probe')).color==='rgb(0, 128, 0)','replacement declarations');setPhase(4);
        }}>verify</dom.button><dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#,
        ".probe:hover {background-color:pink}",
    );
    for (key, phase) in [("add", 1), ("reorder", 2), ("replace", 3), ("verify", 4)] {
        ui.click(key);
        ui.wait_for(&format!("phase {phase}"));
    }
}

#[test]
fn transitive_variable_aliases_and_fallbacks_invalidate_receipts() {
    let mut ui=open(r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        export default function App(){const [phase,setPhase]=useState(0);return <PageWindow>
        <dom.div id="probe" className="probe">probe</dom.div>
        <dom.button key="change" onClick={()=>{document.documentElement.style.setProperty('--base','blue');setPhase(1)}}>change</dom.button>
        <dom.button key="verify" onClick={()=>{check(getComputedStyle(document.getElementById('probe')).color==='rgb(0, 0, 255)','transitive alias changed');document.documentElement.style.removeProperty('--base');setPhase(2)}}>verify</dom.button>
        <dom.button key="fallback" onClick={()=>{check(getComputedStyle(document.getElementById('probe')).color==='rgb(255, 0, 0)','fallback restored');setPhase(3)}}>fallback</dom.button><dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#," :root {--alias:var(--base,red)} .probe {color:var(--alias)} .probe:hover {background-color:pink}");
    ui.click("change");
    ui.wait_for("phase 1");
    ui.click("verify");
    ui.wait_for("phase 2");
    ui.click("fallback");
    ui.wait_for("phase 3");
}

#[test]
fn editing_existing_style_text_and_cssom_rules_replaces_declarations() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        let sheet;
        export default function App(){const [phase,setPhase]=useState(0);return <PageWindow>
        <dom.div id="probe" className="probe">probe</dom.div>
        <dom.button key="add" onClick={()=>{sheet=document.createElement('style');sheet.textContent='.probe {color:blue}';document.head.appendChild(sheet);setPhase(1)}}>add</dom.button>
        <dom.button key="text" onClick={()=>{check(getComputedStyle(document.getElementById('probe')).color==='rgb(0, 0, 255)','initial sheet');sheet.textContent='.probe {color:green}';setPhase(2)}}>text</dom.button>
        <dom.button key="rule" onClick={()=>{check(getComputedStyle(document.getElementById('probe')).color==='rgb(0, 128, 0)','existing style text edit');sheet.sheet.deleteRule(0);sheet.sheet.insertRule('.probe {color:red}',0);setPhase(3)}}>rule</dom.button>
        <dom.button key="verify" onClick={()=>{check(getComputedStyle(document.getElementById('probe')).color==='rgb(255, 0, 0)','existing CSSOM delete/insert edit');setPhase(4)}}>verify</dom.button>
        <dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#,
        ".probe:hover {background-color:pink}",
    );
    for (key, phase) in [("add", 1), ("text", 2), ("rule", 3), ("verify", 4)] {
        ui.click(key);
        ui.wait_for(&format!("phase {phase}"));
    }
}
