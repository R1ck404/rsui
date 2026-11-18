//! Computed styles preserve fresh wrapper isolation and lazy input snapshots.

mod hands;
use std::sync::atomic::{AtomicU32, Ordering};

fn open(source: &str, css: &str) -> hands::Hands {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-computed-styles-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("app.css"), format!("body {{ margin: 0; }}\n{css}")).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, format!("import './app.css';\nfunction check(ok, why) {{ if (!ok) throw new Error(why); }}\n{source}")).unwrap();
    hands::Hands::open(&file, (500, 350))
}

#[test]
fn fresh_wrappers_local_setters_and_custom_rects_are_isolated() {
    let mut ui = open(r#"
        import { PageWindow, dom } from 'rsui/web';
        import { useState } from 'react';
        export default function App() {
            const [said,setSaid] = useState('waiting');
            return <PageWindow><dom.div id="box" style={{width:100,height:20,border:'1px solid black',fontSize:14}} />
                <dom.div id="hidden-parent" style={{visibility:'hidden',height:1}} />
                <dom.button key="run" onClick={() => {
                    const box=document.getElementById('box');
                    box.getBoundingClientRect();
                    const a=getComputedStyle(box), b=getComputedStyle(box);
                    check(a!==b,'fresh wrappers');
                    void a.color; void b.color; void a.width; void b.width;
                    a.borderTopColor='rgb(1, 2, 3)';
                    check(a.borderColor==='rgb(1, 2, 3)','local shorthand sees local override');
                    check(b.borderColor!==a.borderColor,'shorthand cache does not leak');
                    a.fontSize='80px'; check(a.font.includes('80px'),'font shorthand sees local override');
                    check(b.fontSize!=='80px','primitive override does not leak');
                    const original=box.getBoundingClientRect;
                    let n=0;
                    box.getBoundingClientRect=()=>({x:0,y:0,left:0,top:0,width:100+(++n),height:20,right:200,bottom:20});
                    const x=getComputedStyle(box).width, y=getComputedStyle(box).width;
                    check(x!==y,'custom bounding rectangles must execute per wrapper');
                    box.getBoundingClientRect=original;
                    const parent=Object.getOwnPropertyDescriptor(box,'parentElement');
                    let fakeParent=document.body;
                    Object.defineProperty(box,'parentElement',{configurable:true,get:()=>fakeParent});
                    check(getComputedStyle(box).visibility==='visible','initial custom parent');
                    fakeParent=document.getElementById('hidden-parent');
                    check(getComputedStyle(box).visibility==='hidden','custom parent getter cannot be cached');
                    if (parent) Object.defineProperty(box,'parentElement',parent); else delete box.parentElement;
                    box.style.setProperty('position','fixed');
                    check(getComputedStyle(box).position==='fixed','CSSOM visible before render');
                    setSaid('isolated passed');
                }}>run</dom.button><dom.p>{said}</dom.p></PageWindow>;
        }
    "#, "");
    ui.click("run");
    ui.wait_for("isolated passed");
}

#[test]
fn a_retained_unread_wrapper_uses_geometry_at_its_first_read() {
    let mut ui = open(r#"
        import { PageWindow, dom } from 'rsui/web'; import { useState } from 'react';
        export default function App() {
            const [said,setSaid]=useState('waiting');
            return <PageWindow><dom.div id="box" className="wide" />
                <dom.button key="capture" onClick={()=>{
                    const box=document.getElementById('box');
                    box.getBoundingClientRect();
                    window.unread=getComputedStyle(box);
                    window.beforeWidth=getComputedStyle(box).width;
                    setSaid('captured');
                }}>capture</dom.button>
                <dom.button key="read" onClick={()=>{
                    const now=getComputedStyle(document.getElementById('box')).width;
                    check(now!==window.beforeWidth,'fixture resized');
                    check(window.unread.width===now,'unread wrapper cannot use expired shared geometry');
                    setSaid('resize passed');
                }}>read</dom.button><dom.p>{said}</dom.p></PageWindow>;
        }
    "#, ".wide {width:50vw;height:20px;}");
    ui.click("capture"); ui.wait_for("captured");
    ui.app.resize((700,350),1.0); ui.settle();
    ui.click("read"); ui.wait_for("resize passed");
}

#[test]
fn lazy_inherited_inputs_are_captured_only_by_properties_that_use_them() {
    let mut ui = open(r#"
        import { PageWindow, dom } from 'rsui/web'; import { useState } from 'react';
        export default function App() {
            const [changed,setChanged]=useState(false), [said,setSaid]=useState('waiting');
            return <PageWindow><dom.div style={{fontFamily:changed?'Courier New':'Arial'}}><dom.div id="box" style={{height:20}} /></dom.div>
                <dom.button key="capture" onClick={()=>{
                    const box=document.getElementById('box'); box.getBoundingClientRect();
                    const warm=getComputedStyle(box);
                    void warm.fontSize; window.beforeFamily=warm.fontFamily; void warm.position;
                    window.late=getComputedStyle(box); void window.late.position;
                    window.frozen=getComputedStyle(box); void window.frozen.fontSize;
                    setChanged(true); setSaid('changed family');
                }}>capture</dom.button>
                <dom.button key="read" onClick={()=>{
                    const now=getComputedStyle(document.getElementById('box')).fontFamily;
                    check(now!==window.beforeFamily,'fixture changed family');
                    check(window.late.fontFamily===now,'position must not capture inherited inputs');
                    check(window.frozen.fontFamily===window.beforeFamily,'fontSize must capture inherited inputs');
                    setSaid('lazy passed');
                }}>read</dom.button><dom.p>{said}</dom.p></PageWindow>;
        }
    "#, "");
    ui.click("capture"); ui.wait_for("changed family");
    ui.click("read"); ui.wait_for("lazy passed");
}

#[test]
fn css_attribute_and_has_updates_invalidate_shared_results() {
    let mut ui = open(r#"
        import { PageWindow, dom } from 'rsui/web'; import { useState } from 'react';
        export default function App() {
            const [on,setOn]=useState(false), [said,setSaid]=useState('waiting');
            return <PageWindow><dom.div id="parent" className="parent"><dom.div id="box" className={on?'box hot':'box'} data-on={on?'yes':undefined} /></dom.div>
                <dom.button key="change" onClick={()=>{
                    const box=document.getElementById('box'), parent=document.getElementById('parent');
                    window.oldPosition=getComputedStyle(box).position;
                    void getComputedStyle(box).position;
                    window.oldBackground=getComputedStyle(parent).backgroundColor;
                    void getComputedStyle(parent).backgroundColor;
                    box.setAttribute('data-extra','changed');
                    box.style.setProperty('transform','translateX(5px)');
                    check(getComputedStyle(box).transform==='matrix(1, 0, 0, 1, 5, 0)','CSSOM read before render');
                    parent.style.setProperty('font-family','serif');
                    // Preflush reads are checked against the uncached oracle too.
                    void getComputedStyle(box).color; void getComputedStyle(parent).position;
                    setOn(true); setSaid('attributes changed');
                }}>change</dom.button>
                <dom.button key="read" onClick={()=>{
                    const box=document.getElementById('box'), parent=document.getElementById('parent');
                    check(box.dataset.extra==='changed','dataset readback');
                    check(getComputedStyle(box).position!==window.oldPosition,'class style updated');
                    check(getComputedStyle(parent).backgroundColor!==window.oldBackground,':has style updated');
                    setSaid('attributes passed');
                }}>read</dom.button><dom.p>{said}</dom.p></PageWindow>;
        }
    "#, ".box {height:20px;} .box.hot {position:absolute;} .parent {background:#112233;} .parent:has([data-on=yes]) {background:#445566;}");
    ui.click("change"); ui.wait_for("attributes changed");
    ui.click("read"); ui.wait_for("attributes passed");
}
