//! Selector-only topology reuse keeps matching active and geometry independent.
mod hands;
use std::sync::atomic::{AtomicU32, Ordering};
fn open(source: &str, css: &str) -> hands::Hands {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "tsx-selector-tree-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("app.css"), format!("body {{margin:0;}}\n{css}")).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(
        &file,
        format!(
            "import './app.css'; function check(ok,why) {{if(!ok) throw new Error(why);}}\n{source}"
        ),
    )
    .unwrap();
    hands::Hands::open(&file, (500, 350))
}

#[test]
fn scroll_and_resize_recheck_has_without_rebuilding_geometry_projection() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import rsui from 'rsui';
        const host=rsui.__host(),full=host.pageTree,kept=host.pageTreeContentKept,inputsKept=host.selectorInputsKept;
        const counts=window.selectorCounts={full:0,kept:0,receipts:0};
        host.pageTree=(...args)=>{counts.full++;return full(...args)};
        host.pageTreeContentKept=()=>{const yes=kept();if(yes)counts.kept++;return yes};
        host.selectorInputsKept=(...args)=>{const yes=inputsKept(...args);if(yes)counts.receipts++;return yes};
        function Inside(){return <dom.div className="target">target</dom.div>}
        export default function App(){return <PageWindow><dom.div className="panel"><Inside/></dom.div>
        <dom.div style={{height:1800}}>tall</dom.div><dom.div style={{position:'fixed',top:0,left:0}}>
        <dom.button key="start" onClick={()=>{window.beforeCounts={full:counts.full,kept:counts.kept,receipts:counts.receipts};window.scrollTo(0,120)}}>start</dom.button>
        <dom.button key="verify" onClick={()=>{
          const a=window.beforeCounts;
          check(counts.kept>a.kept || counts.receipts>a.receipts,'scroll did not recheck retained selector topology');
          console.error('scroll selector projection full='+(counts.full-a.full)+' kept='+(counts.kept-a.kept)+' receipts='+(counts.receipts-a.receipts));
          check(getComputedStyle(document.querySelector('.panel')).backgroundColor==='rgb(0, 128, 0)',':has still matches');
          document.getElementById('done').textContent='scroll passed';
        }}>verify</dom.button><dom.p id="done">waiting</dom.p></dom.div></PageWindow>}
    "#,
        ".panel {background:red}.panel:has(.target){background:green}",
    );
    ui.click("start");
    ui.app.resize((501, 350), 1.0);
    ui.settle();
    ui.click("verify");
    ui.wait_for("scroll passed");
}

#[test]
fn live_attribute_class_and_validity_changes_are_observed() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        function Inside(){return <dom.input id="field" className="field" required defaultValue="good"/>}
        export default function App(){const [phase,setPhase]=useState(0);return <PageWindow>
        <dom.div id="panel" className="panel"><Inside/></dom.div>
        <dom.button key="change" onClick={()=>{const f=document.getElementById('field');f.setAttribute('data-hot','yes');f.classList.add('hot');f.setCustomValidity('bad');setPhase(1)}}>change</dom.button>
        <dom.button key="verify" onClick={()=>{const p=document.getElementById('panel');check(getComputedStyle(p).backgroundColor==='rgb(0, 128, 0)','attributes and invalidity must recheck');setPhase(2)}}>verify</dom.button><dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#,
        ".panel{background:red}.panel:has(.field.hot[data-hot=yes]:invalid){background:green}",
    );
    ui.click("change");
    ui.wait_for("phase 1");
    ui.click("verify");
    ui.wait_for("phase 2");
}

#[test]
fn structural_text_and_nth_mutations_refresh_topology() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        function Inside({phase}){return <>{phase===0?<dom.span className="a">first</dom.span>:null}<dom.span className="b">{phase===0?'': 'text'}</dom.span></>}
        export default function App(){const [phase,setPhase]=useState(0);return <PageWindow>
        <dom.div id="panel" className="panel"><Inside phase={phase}/></dom.div>
        <dom.button key="change" onClick={()=>setPhase(1)}>change</dom.button>
        <dom.button key="verify" onClick={()=>{const p=document.getElementById('panel');check(getComputedStyle(p).backgroundColor==='rgb(0, 128, 0)','child removal, nth and empty text must refresh');setPhase(2)}}>verify</dom.button><dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#,
        ".panel{background:red}.panel:has(.b:first-child:not(:empty)){background:green}",
    );
    ui.click("change");
    ui.wait_for("phase 1");
    ui.click("verify");
    ui.wait_for("phase 2");
}

#[test]
fn selector_checks_do_not_advance_geometry_anchor() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';import rsui from 'rsui';
        function Inside(){return <dom.span className="target">target</dom.span>}
        export default function App(){const [on,setOn]=useState(false),[said,setSaid]=useState('waiting');return <PageWindow>
        <dom.div id="panel" className="panel" style={{position:'fixed',top:30,left:20,transform:on?'translateX(15px)':'translateX(0px)'}}><Inside/></dom.div>
        <dom.button key="change" onClick={()=>{window.oldRect=document.getElementById('panel').getBoundingClientRect().left;setOn(true)}}>change</dom.button>
        <dom.button key="verify" onClick={()=>{const p=document.getElementById('panel');void getComputedStyle(p).backgroundColor;const host=rsui.__host();host.pageTreeContentKept();
          check(Math.abs(p.getBoundingClientRect().left-window.oldRect-15)<0.01,'selector reuse cannot certify stale geometry');setSaid('geometry passed')
        }}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#,
        ".panel:has(.target){background:green}",
    );
    ui.click("change");
    ui.click("verify");
    ui.wait_for("geometry passed");
}

#[test]
fn native_to_styled_ownership_and_portal_membership_refresh_live_descriptors() {
    let mut ui = open(
        r#"
        import {PageWindow,dom,__elementPortal} from 'rsui/web';import {useState,useLayoutEffect} from 'react';
        function Inside(){return <dom.span>inside</dom.span>}
        function make(){const n=document.createElement('span');n.className='owned';n.textContent='owned';return n}
        export default function App(){const [phase,setPhase]=useState(0),[target,setTarget]=useState(null);
          useLayoutEffect(()=>{const p=document.getElementById('panel');p.appendChild(make());setTarget(p)},[]);
          return <PageWindow><dom.div id="panel" className="panel"><Inside/></dom.div>
          {target && phase>=2?__elementPortal(<dom.span className="portal">portal</dom.span>,target):null}
          <dom.button key="warm" onClick={()=>{window.owned=make();target.appendChild(window.owned);setPhase(1)}}>warm</dom.button>
          <dom.button key="change" onClick={()=>{void getComputedStyle(target).backgroundColor;window.owned.id='now-styled';setPhase(2)}}>change</dom.button>
          <dom.button key="verify" onClick={()=>{check(getComputedStyle(target).backgroundColor==='rgb(0, 128, 0)','live owner replacement and portal membership');setPhase(3)}}>verify</dom.button><dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#,
        ".panel{background:red}.panel:has(#now-styled):has(.portal){background:green}",
    );
    ui.click("warm");
    ui.wait_for("phase 1");
    ui.click("change");
    ui.wait_for("phase 2");
    ui.click("verify");
    ui.wait_for("phase 3");
}

#[test]
#[ignore = "Baseline ancestor-only :has(:focus) fails even without topology reuse; separate focus dependency issue"]
fn focus_state_changes_are_visible_inside_has() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState,useLayoutEffect} from 'react';
        function Inside(){return <dom.input id="field" className="field"/>}
        export default function App(){const [phase,setPhase]=useState(0);useLayoutEffect(()=>{if(phase===1){document.getElementById('field').focus();setTimeout(()=>{check(getComputedStyle(document.getElementById('panel')).backgroundColor==='rgb(0, 128, 0)','focus state inside has');setPhase(2)},25)}},[phase]);
          return <PageWindow><dom.div id="panel" className="panel"><Inside/></dom.div>
          <dom.button key="focus" onClick={()=>setPhase(1)}>focus</dom.button><dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#,
        ".panel{background:red}.panel:has(.field:focus){background:green}",
    );
    ui.click("focus");
    ui.wait_for("phase 1");
    ui.wait_for("phase 2");
}

// A direct focus rule registers the baseline focus dependency; keep the unsupported
// ancestor-only case above as its own unchanged ignored regression.
#[test]
fn tracked_focus_state_changes_are_visible_inside_has() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState,useLayoutEffect} from 'react';
        function Inside(){return <dom.input id="field" className="field"/>}
        export default function App(){const [phase,setPhase]=useState(0);useLayoutEffect(()=>{if(phase===1){document.getElementById('field').focus();setTimeout(()=>{check(getComputedStyle(document.getElementById('panel')).backgroundColor==='rgb(0, 128, 0)','focus state inside has');setPhase(2)},25)}},[phase]);
          return <PageWindow><dom.div id="panel" className="panel"><Inside/></dom.div>
          <dom.button key="focus" onClick={()=>setPhase(1)}>focus</dom.button><dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#,
        ".panel{background:red}.panel:has(.field:focus){background:green}.field:focus{opacity:0.75}",
    );
    ui.click("focus");
    ui.wait_for("phase 1");
    ui.wait_for("phase 2");
}

#[test]
fn repeated_style_reads_in_the_current_tree_skip_native_content_comparison() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import rsui from 'rsui';import {useState} from 'react';
        function Inside(){return <dom.span className="target">target</dom.span>}
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow>
        <dom.div id="panel" className="panel"><Inside/></dom.div>
        <dom.button key="verify" onClick={()=>{
            const panel=document.getElementById('panel');
            check(getComputedStyle(panel).backgroundColor==='rgb(0, 128, 0)','fixture has matching style');
            const host=rsui.__host(),original=host.pageTreeContentKept;let calls=0;
            host.pageTreeContentKept=(...args)=>{calls++;return original(...args)};
            try {
                for(let i=0;i<25;i++)check(getComputedStyle(panel).backgroundColor==='rgb(0, 128, 0)','repeat style result');
                check(calls===0,'already checked selector lives should not compare native tree: '+calls);
            } finally {host.pageTreeContentKept=original}
            setSaid('repeat passed');
        }}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#,
        ".panel{background:red}.panel:has(.target){background:green}",
    );
    ui.click("verify");
    ui.wait_for("repeat passed");
}
