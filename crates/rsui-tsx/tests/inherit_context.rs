//! Explicit inheritance follows current parent values and preserves local declarations.
mod hands;
use std::sync::atomic::{AtomicU32, Ordering};
fn open(source: &str, css: &str) -> hands::Hands {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "tsx-inherit-context-{}-{}",
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
fn parent_transform_and_border_changes_preserve_native_descendant_styles() {
    let mut ui = open(
        r#"
        import {PageWindow,dom,__styled as styled} from 'rsui/web';import {useState,useLayoutEffect} from 'react';
        const Parent=styled.div``;
        function makeChild(id){const node=document.createElement('div');node.className='native-probe';node.textContent='child';node.style.width='33px';node.style.height='12px';node.style.letterSpacing='1.234px';return node}
        const seed=<dom.div style={{width:'inherit'}}>seed</dom.div>;
        export default function App(){const [phase,setPhase]=useState(0);
        useLayoutEffect(()=>{document.getElementById('parent').appendChild(makeChild('child'))},[]);
        return <PageWindow>
        <Parent id="parent" style={{width:120,transform:phase===1?'translateX(2px)':'translateX(1px)',border:phase===1?'3px solid red':'2px solid blue'}}>{seed}</Parent>
        <dom.button key="warm" onClick={()=>{document.getElementById('parent').appendChild(makeChild('second'));setPhase(10)}}>warm</dom.button>
        <dom.button key="change" onClick={()=>setPhase(1)}>change</dom.button>
        <dom.button key="verify" onClick={()=>{
          const childStyle=getComputedStyle(document.getElementById('parent').querySelector('.native-probe'));
          check(childStyle.width==='33px','child geometry preserved');
          check(childStyle.letterSpacing==='1.234px','child local style preserved');setPhase(2)}}>verify</dom.button>
        <dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#,
        "",
    );
    ui.click("warm");
    ui.wait_for("phase 10");
    ui.click("change");
    ui.wait_for("phase 1");
    ui.click("verify");
    ui.wait_for("phase 2");
}

#[test]
fn inherited_width_changes_propagate() {
    let mut ui = open(
        r#"
        import {PageWindow,dom,__styled as styled} from 'rsui/web';import {useState} from 'react';const Parent=styled.div``;
        const child=<dom.div id="child" style={{width:'inherit',height:12}}>child</dom.div>;
        export default function App(){const [wide,setWide]=useState(false);const [said,setSaid]=useState('waiting');return <PageWindow>
        <Parent style={{width:wide?177:120}}>{child}</Parent><dom.button key="change" onClick={()=>setWide(true)}>change</dom.button>
        <dom.button key="verify" onClick={()=>{check(getComputedStyle(document.getElementById('child')).width==='177px','inherited width must update');setSaid('width passed')}}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#,
        "",
    );
    ui.click("change");
    ui.click("verify");
    ui.wait_for("width passed");
}

#[test]
fn first_new_inherit_property_reads_latest_parent_snapshot_immediately() {
    let mut ui = open(
        r#"
        import {PageWindow,dom,__styled as styled} from 'rsui/web';import {useState,useLayoutEffect} from 'react';const Parent=styled.div``;
        function Late(){const [take,setTake]=useState(false);const [said,setSaid]=useState('waiting');
          useLayoutEffect(()=>{if(take){check(getComputedStyle(document.getElementById('late')).opacity==='0.75','first layout effect sees latest unobserved property');setSaid('immediate passed')}},[take]);
          return <><dom.div id="late" style={{width:'inherit',opacity:take?'inherit':1}}>late</dom.div><dom.button key="take" onClick={()=>setTake(true)}>take</dom.button><dom.p>{said}</dom.p></>}
        const late=<Late/>;
        export default function App(){const [changed,setChanged]=useState(false);return <PageWindow>
        <Parent style={{width:120,opacity:changed?0.75:0.25,transform:changed?'translateX(2px)':'translateX(1px)'}}>{late}</Parent>
        <dom.button key="parent" onClick={()=>setChanged(true)}>parent</dom.button></PageWindow>}
    "#,
        "",
    );
    ui.click("parent");
    ui.click("take");
    ui.wait_for("immediate passed");
}

#[test]
fn class_and_has_changes_update_inherited_values() {
    let mut ui = open(
        r#"
        import {PageWindow,dom,__styled as styled} from 'rsui/web';import {useState} from 'react';const Parent=styled.div``;
        const child=<dom.div id="child" style={{width:'inherit',height:12}}>child</dom.div>;
        export default function App(){const [on,setOn]=useState(false);const [said,setSaid]=useState('waiting');return <PageWindow>
        <Parent className={on?'parent big':'parent'}>{child}{on?<dom.span className="present">present</dom.span>:null}</Parent>
        <dom.button key="change" onClick={()=>setOn(true)}>change</dom.button><dom.button key="verify" onClick={()=>{check(getComputedStyle(document.getElementById('child')).width==='166px','class and has cascade');setSaid('cascade passed')}}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#,
        ".parent{width:120px;transition:opacity 1ms}.parent.big:has(.present){width:166px}",
    );
    ui.click("change");
    ui.click("verify");
    ui.wait_for("cascade passed");
}

#[test]
fn root_inherit_fallback_accepts_raw_page_look_and_refreshes() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        export default function App(){const [phase,setPhase]=useState(0);return <PageWindow>
        <dom.div id="top" style={{width:'inherit',height:12}}>top</dom.div>
        <dom.button key="change" onClick={()=>{
            check(getComputedStyle(document.getElementById('top')).width==='91px','null context uses raw root look');
            const s=document.createElement('style');s.textContent='html {width:117px}';document.head.appendChild(s);setPhase(1);
        }}>change</dom.button><dom.button key="verify" onClick={()=>{check(getComputedStyle(document.getElementById('top')).width==='117px','raw root look refresh');setPhase(2)}}>verify</dom.button><dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#,
        "html {width:91px}",
    );
    ui.click("change");
    ui.wait_for("phase 1");
    ui.click("verify");
    ui.wait_for("phase 2");
}
