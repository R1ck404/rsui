//! Selector input receipts must preserve real CSS behavior and unsafe fallbacks.
mod hands;

fn open(name: &str, body: &str) -> hands::Hands {
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("has-input-receipts-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("app.css"),
        "body{margin:0}.panel{background:red}.panel:has(>[data-hot=on]){background:green}").unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, format!(r#"
        import {{PageWindow,dom}} from 'rsui/web';import rsui from 'rsui';import {{useState}} from 'react';import './app.css';
        function check(ok,why){{if(!ok)throw new Error(why)}}
        const host=rsui.__host(),kept=host.selectorInputsKept,capture=host.captureSelectorInputs;
        window.receiptCounts={{hits:0,declined:0,captures:0,safe:0,programs:0}};
        const safePrograms=host.selectorReceiptSafePrograms;
        host.selectorReceiptSafePrograms=(...args)=>{{window.receiptCounts.programs++;const yes=safePrograms(...args);if(yes)window.receiptCounts.safe++;return yes}};
        host.selectorInputsKept=(...args)=>{{const yes=kept(...args);if(yes)window.receiptCounts.hits++;return yes}};
        host.captureSelectorInputs=(...args)=>{{window.receiptCounts.captures++;const value=capture(...args);if(value===undefined)window.receiptCounts.declined++;return value}};
        function Inside(){{return <dom.div id="child" data-hot="off">child</dom.div>}}
        function color(){{return getComputedStyle(document.getElementById('panel')).backgroundColor}}
        function recheck(){{check(typeof globalThis.__rsuiLaidOut==='function','layout observer registered');globalThis.__rsuiLaidOut()}}
        {body}
    "#)).unwrap();
    hands::Hands::open(&file, (500, 350))
}

#[test]
fn stable_scroll_and_resize_reuse_without_losing_live_attribute_changes() {
    let mut ui = open("stable", r#"
        export default function App(){const[said,setSaid]=useState('waiting');return <PageWindow>
          <dom.div id="panel" className="panel"><Inside/></dom.div><dom.div style={{height:2000}}/>
          <dom.div style={{position:'fixed',top:0,left:0}}><dom.button key="scroll" onClick={()=>{window.beforeHits=window.receiptCounts.hits;window.scrollTo(0,100);recheck();recheck()}}>scroll</dom.button>
          <dom.button key="change" onClick={()=>{check(color()==='rgb(255, 0, 0)','initial false recipe');
            console.error('receipt counts '+JSON.stringify(window.receiptCounts)+' intrinsics='+host.selectorReceiptSafeIntrinsics()+' owners='+Array.from(document.querySelectorAll('*'),x=>x.__live?.hasDrawn?.size??0).join(','));
            if(process.env.RSUI_HAS_INPUT_RECEIPTS==='1')check(window.receiptCounts.hits>0,'must exercise an actual native certificate');
            document.getElementById('child').setAttribute('data-hot','on')}}>change</dom.button>
          <dom.button key="verify" onClick={()=>{check(color()==='rgb(0, 128, 0)','imperative attribute invalidates');setSaid('stable passed')}}>verify</dom.button>
          <dom.p>{said}</dom.p></dom.div></PageWindow>}
    "#);
    ui.click("scroll");
    for width in [501, 502, 503] { ui.app.resize((width,350),1.0); ui.settle(); }
    ui.click("change"); ui.click("verify"); ui.wait_for("stable passed");
}

#[test]
fn in_place_live_data_and_undefined_attribute_override_are_not_hidden() {
    let mut ui = open("live-data", r#"
        export default function App(){const[said,setSaid]=useState('waiting');return <PageWindow>
          <dom.div id="panel" className="panel"><Inside/></dom.div><dom.div style={{height:2000}}/>
          <dom.div style={{position:'fixed',top:0,left:0}}><dom.button key="change" onClick={()=>{
            const child=document.getElementById('child');child.__live.given['data-hot']='on';recheck();
            check(color()==='rgb(0, 128, 0)','in-place primitive mutation');
            child.__live.attributes={'data-hot':undefined};recheck();
            check(color()==='rgb(255, 0, 0)','own undefined masks given');setSaid('data passed')}}>change</dom.button>
          <dom.p>{said}</dom.p></dom.div></PageWindow>}
    "#);
    ui.click("change"); ui.wait_for("data passed");
}

#[test]
fn self_removing_getter_cannot_certify_its_post_matching_value() {
    let mut ui = open("getter", r#"
        export default function App(){const[said,setSaid]=useState('waiting');return <PageWindow>
          <dom.div id="panel" className="panel"><Inside/></dom.div><dom.div style={{height:2000}}/>
          <dom.div style={{position:'fixed',top:0,left:0}}><dom.button key="change" onClick={()=>{
            window.getterReads=0;const given=document.getElementById('child').__live.given;
            Object.defineProperty(given,'data-hot',{enumerable:true,configurable:true,get(){window.getterReads++;
              Object.defineProperty(given,'data-hot',{enumerable:true,configurable:true,writable:true,value:'off'});return'on'}});
            window.scrollTo(0,100);recheck()}}>change</dom.button>
          <dom.button key="again" onClick={()=>{window.scrollTo(0,150);recheck()}}>again</dom.button>
          <dom.button key="verify" onClick={()=>{check(window.getterReads>0,'baseline getter executed');
            check(color()==='rgb(255, 0, 0)','getter result must not be pinned by post-only capture');setSaid('getter passed')}}>verify</dom.button>
          <dom.p>{said}</dom.p></dom.div></PageWindow>}
    "#);
    ui.click("change"); ui.settle(); ui.click("again"); ui.settle(); ui.click("verify"); ui.wait_for("getter passed");
}

#[test]
fn changed_standard_string_conversion_forces_original_matching() {
    let mut ui = open("intrinsic", r#"
        export default function App(){const[said,setSaid]=useState('waiting');return <PageWindow>
          <dom.div id="panel" className="panel"><Inside/></dom.div><dom.div style={{height:2000}}/>
          <dom.div style={{position:'fixed',top:0,left:0}}><dom.button key="prime" onClick={()=>{
            document.getElementById('child').setAttribute('data-hot','on');window.scrollTo(0,80);recheck()}}>prime</dom.button>
          <dom.button key="change" onClick={()=>{check(color()==='rgb(0, 128, 0)','prime true');
            window.originalString=globalThis.String;globalThis.String=function(value){return value==='on'?'off':window.originalString(value)};
            window.scrollTo(0,120);recheck()}}>change</dom.button>
          <dom.button key="verify" onClick={()=>{const result=color();globalThis.String=window.originalString;
            check(result==='rgb(255, 0, 0)','overridden conversion requires fallback');setSaid('intrinsic passed')}}>verify</dom.button>
          <dom.p>{said}</dom.p></dom.div></PageWindow>}
    "#);
    ui.click("prime"); ui.settle(); ui.click("change"); ui.settle(); ui.click("verify"); ui.wait_for("intrinsic passed");
}
