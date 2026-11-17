//! CSS declaration refreshes preserve current variables, conditions and cascade ordering.

mod hands;
use std::sync::atomic::{AtomicU32, Ordering};

fn open(source: &str, css: &str) -> hands::Hands {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "tsx-sheet-syntax-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("app.css"), format!("body {{margin:0;}}\n{css}")).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, format!("import './app.css'; function check(ok,why){{if(!ok)throw new Error(why);}}\n{source}")).unwrap();
    let mut ui = hands::Hands::open(&file, (500, 350));
    // Announce the native viewport to CSS as well as constructing the app at this size.
    ui.app.resize((500, 350), 1.0);
    ui.settle();
    ui
}

#[test]
fn declaration_refresh_updates_variables_and_sheet_replacement_cascade() {
    let mut ui = open(r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow>
        <dom.div id="box" className="syntax-probe" data-ready="yes">probe</dom.div>
        <dom.button key="change" onClick={()=>{
          check(getComputedStyle(document.getElementById('box')).width==='71px','initial width');
          document.body.style.setProperty('--syntax-width','97px');
          document.body.classList.add('expanded');setSaid('changed');
        }}>change</dom.button>
        <dom.button key="verify" onClick={()=>{
          const look=getComputedStyle(document.getElementById('box'));
          check(look.width==='97px','fresh variable value');check(look.height==='32px','fresh cascade');
          const style=document.createElement('style');style.id='late-sheet';
          style.textContent='.syntax-probe[data-ready] {width:113px}';document.head.appendChild(style);setSaid('appended');
        }}>verify</dom.button>
        <dom.button key="replace" onClick={()=>{
          check(getComputedStyle(document.getElementById('box')).width==='113px','new sheet cascade');
          document.getElementById('late-sheet').textContent='.syntax-probe[data-ready] {width:127px}';setSaid('replaced');
        }}>replace</dom.button>
        <dom.button key="finish" onClick={()=>{
          check(getComputedStyle(document.getElementById('box')).width==='127px','replacement drops old syntax');setSaid('passed');
        }}>finish</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#, ":root{--syntax-width:71px}.syntax-probe[data-ready]{width:var(--syntax-width);height:1rem}.expanded .syntax-probe[data-ready]{height:2rem}");
    ui.click("change"); ui.wait_for("changed");
    ui.click("verify"); ui.wait_for("appended");
    ui.click("replace"); ui.wait_for("replaced");
    ui.click("finish"); ui.wait_for("passed");
}

#[test]
fn declaration_refresh_preserves_media_viewport_rem_and_structural_selectors() {
    let mut ui = open(r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow>
        <dom.div><dom.div id="box" className="syntax-probe" data-ready="yes" /><dom.div>second</dom.div></dom.div>
        <dom.button key="font" onClick={()=>{
          const look=getComputedStyle(document.getElementById('box'));
          check(look.width==='50px','initial media: '+look.width);check(look.height==='16px','initial rem: '+look.height);
          const style=document.createElement('style');style.textContent='html {font-size:20px}';document.head.appendChild(style);setSaid('font changed');
        }}>font</dom.button>
        <dom.button key="verify" onClick={()=>{
          const look=getComputedStyle(document.getElementById('box'));
          check(look.width==='70px','resized viewport and media');check(look.height==='40px','rem refresh');
          check(look.paddingTop==='3px','structural requirement retained');setSaid('passed');
        }}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#, ".syntax-probe[data-ready]{box-sizing:border-box;width:10vw;height:1rem}.syntax-probe:first-child{padding-top:3px}@media(min-width:600px){.syntax-probe[data-ready]{height:2rem}}");
    ui.click("font"); ui.wait_for("font changed");
    ui.app.resize((700, 350), 1.0); ui.settle();
    ui.click("verify"); ui.wait_for("passed");
}
