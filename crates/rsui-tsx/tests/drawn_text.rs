//! Literal DOM children preserve identity without reading the complete rendered tree.
mod hands;
use std::sync::atomic::{AtomicU32, Ordering};
fn open(source: &str, css: &str) -> hands::Hands {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "tsx-drawn-text-{}-{}",
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
fn reading_literal_text_nodes_needs_no_native_page_tree() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';import rsui from 'rsui';
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow>
        <dom.div id="box" style={{textTransform:'uppercase'}}>hello</dom.div><dom.div id="number">{42}</dom.div>
        <dom.button key="verify" onClick={()=>{
            const box=document.getElementById('box'), number=document.getElementById('number'),host=rsui.__host(),original=host.pageTree;
            let calls=0;host.pageTree=(...args)=>{calls++;return original(...args)};
            try {
                const one=box.childNodes[0];check(one.nodeType===3 && one.data==='hello','DOM text uses uncased source');
                check(box.childNodes.length===1 && box.firstChild===one,'text node identity');
                check(number.childNodes[0].data==='42','numeric DOM text');
                const extra=document.createElement('span');extra.textContent='extra';box.appendChild(extra);
                check(box.childNodes.length===2 && box.childNodes[0]===one && box.lastChild===extra,'imperative children remain after text');
                check(calls===0,'literal DOM reads must not serialize page tree: '+calls);
            } finally {host.pageTree=original}
            setSaid('literal passed');
        }}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#,
        "",
    );
    ui.click("verify");
    ui.wait_for("literal passed");
}

#[test]
fn literal_updates_keep_text_identity_and_element_transitions_refresh_children() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        export default function App(){const [phase,setPhase]=useState(0);return <PageWindow>
        <dom.div id="box">{phase<2?(phase===0?'first':'second'):<dom.span id="inner">inner</dom.span>}</dom.div>
        <dom.button key="text" onClick={()=>{window.keptText=document.getElementById('box').firstChild;setPhase(1)}}>text</dom.button>
        <dom.button key="element" onClick={()=>{const box=document.getElementById('box');check(box.firstChild===window.keptText && box.firstChild.data==='second','text update preserves node');setPhase(2)}}>element</dom.button>
        <dom.button key="verify" onClick={()=>{const box=document.getElementById('box');check(box.childNodes.length===1 && box.firstChild===document.getElementById('inner'),'text to element transition');setPhase(3)}}>verify</dom.button><dom.p>{'phase '+phase}</dom.p></PageWindow>}
    "#,
        "",
    );
    ui.click("text");
    ui.wait_for("phase 1");
    ui.click("element");
    ui.wait_for("phase 2");
    ui.click("verify");
    ui.wait_for("phase 3");
}

#[test]
fn portals_and_rendered_element_order_keep_the_full_dom_path() {
    let mut ui = open(
        r#"
        import {PageWindow,dom,__elementPortal} from 'rsui/web';import {createPortal} from 'react-dom';import {useState,useLayoutEffect} from 'react';
        export default function App(){const [target,setTarget]=useState(null),[said,setSaid]=useState('waiting');
        useLayoutEffect(()=>setTarget(document.getElementById('box')),[]);
        return <PageWindow><dom.div id="box">literal</dom.div>{target?createPortal(' portal text',target):null}{target?__elementPortal(<dom.span id="portal">portal</dom.span>,target):null}
        <dom.div id="order"><dom.span id="a">a</dom.span><dom.span id="b">b</dom.span></dom.div>
        <dom.button key="verify" onClick={()=>{
            const box=document.getElementById('box'),portal=document.getElementById('portal');
            // Public react-dom uses ElementPortal only for text; call the helper
            // directly as well to cover a rendered element in its portalKids path.
            check(box.childNodes.some(node=>node.nodeType===3 && node.data===' portal text'),'public text portal retained');
            check(box.childNodes.some(node=>node===portal),'element portal retained beside literal text');
            const order=document.getElementById('order'),a=document.getElementById('a'),b=document.getElementById('b');
            order.insertBefore(b,a);check(order.firstChild===b && order.lastChild===a,'imperative rendered child reorder');setSaid('portal passed');
        }}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#,
        "",
    );
    ui.click("verify");
    ui.wait_for("portal passed");
}

#[test]
fn child_topology_survives_geometry_changes_without_certifying_old_rectangles() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import rsui from 'rsui';
        export default function App(){return <PageWindow>
        <dom.div id="box" style={{position:'fixed',top:80,left:20,transform:'translateX(0px)'}}><dom.span id="child">child</dom.span></dom.div>
        <dom.button key="move" onClick={()=>{
            const box=document.getElementById('box');window.firstChild=box.firstChild;
            window.firstLeft=window.firstChild.getBoundingClientRect().left;
            // Warm DOM ownership before changing geometry only.
            void box.childNodes;
            window.firstTree=rsui.__host().treeNow();box.style.transform='translateX(25px)';
            setTimeout(()=>{
            const box=document.getElementById('box'),host=rsui.__host(),original=host.pageTree;
            check(host.treeNow()!==window.firstTree,'fixture changed native tree');
            check(host.pageTreeContentKept()===true,'fixture retained DOM content');
            const geometryWasKept=host.pageTreeKept();
            let calls=0;host.pageTree=(...args)=>{calls++;return original(...args)};
            try{
                check(box.childNodes.length===1 && box.firstChild===window.firstChild,'element identity after movement');
                check(calls===0,'topology-only reads should not serialize page: '+calls);
                check(Math.abs(box.firstChild.getBoundingClientRect().left-window.firstLeft-25)<0.01,'child geometry refreshes independently');
                check(host.pageTreeKept()===geometryWasKept,'child reads must not advance the geometry anchor');
            }finally{host.pageTree=original}
            document.getElementById('done').textContent='topology passed';
            },25);
        }}>move</dom.button><dom.p id="done">waiting</dom.p></PageWindow>}
    "#,
        "",
    );
    ui.click("move");
    ui.wait_for("topology passed");
}

#[test]
fn topology_refreshes_after_child_changes_portals_and_native_owner_changes() {
    let mut ui = open(
        r#"
        import {PageWindow,dom,__elementPortal} from 'rsui/web';import {useState,useLayoutEffect} from 'react';
        function Inside({changed}){return changed?<dom.strong id="new">new</dom.strong>:<dom.span id="old">old</dom.span>}
        export default function App(){const [changed,setChanged]=useState(false),[target,setTarget]=useState(null);
            useLayoutEffect(()=>setTarget(document.getElementById('box')),[]);
            return <PageWindow><dom.div id="box"><Inside changed={changed}/><dom.span id="tail">tail</dom.span></dom.div>
            {target && changed?__elementPortal(<dom.span id="portal">portal</dom.span>,target):null}
            <dom.button key="warm" onClick={()=>{
                const box=document.getElementById('box');void box.childNodes;
                const owned=document.createElement('span');owned.className='owned';owned.textContent='owned';
                box.appendChild(owned);window.owned=owned;
            }}>warm</dom.button>
            <dom.button key="change" onClick={()=>{
                void target.childNodes;window.owned.id='now-styled';setChanged(true);
            }}>change</dom.button>
            <dom.button key="verify" onClick={()=>{
                const box=document.getElementById('box'),fresh=document.getElementById('new'),tail=document.getElementById('tail'),portal=document.getElementById('portal');
                check(box.childNodes.includes(fresh) && !box.childNodes.some(n=>n.id==='old'),'replaced rendered child');
                check(box.childNodes.includes(portal),'new portal child');
                check(box.childNodes.includes(window.owned) && window.owned.id==='now-styled','native owner still uses live DOM handle');
                box.insertBefore(tail,fresh);check(box.firstChild===tail,'imperative rendered ordering refreshes');
                document.getElementById('done').textContent='owners passed';
            }}>verify</dom.button><dom.p id="done">waiting</dom.p></PageWindow>}
    "#,
        ".owned {color:red} #now-styled {color:green}",
    );
    ui.click("warm");
    ui.click("change");
    ui.click("verify");
    ui.wait_for("owners passed");
}

#[test]
fn hidden_children_follow_their_current_parent() {
    let mut ui = open(
        r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        function Hidden(){return <dom.span id="hidden" style={{display:'none'}}>hidden text</dom.span>}
        export default function App(){const [right,setRight]=useState(false);return <PageWindow>
            <dom.div id="left">{!right?<Hidden/>:null}</dom.div><dom.div id="right">{right?<Hidden/>:null}</dom.div>
            <dom.button key="move" onClick={()=>{
                const left=document.getElementById('left'),hidden=document.getElementById('hidden');
                check(left.childNodes.includes(hidden),'initial hidden membership');setRight(true);
            }}>move</dom.button>
            <dom.button key="verify" onClick={()=>{
                const left=document.getElementById('left'),right=document.getElementById('right'),hidden=document.getElementById('hidden');
                check(!left.childNodes.includes(hidden) && right.childNodes.includes(hidden),'moved hidden membership');
                check(hidden.parentElement===right,'hidden current parent');
                document.getElementById('done').textContent='hidden passed';
            }}>verify</dom.button><dom.p id="done">waiting</dom.p></PageWindow>}
    "#,
        "",
    );
    ui.click("move");
    ui.click("verify");
    ui.wait_for("hidden passed");
}
