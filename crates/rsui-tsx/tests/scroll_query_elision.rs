//! Scroll adjustment queries are needed only for pending requests or whole offsets.
mod hands;

fn open(name: &str, body: &str) -> hands::Hands {
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-scroll-query-elision-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("app.css"), "body {margin:0}").unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, format!(r#"
        import {{PageWindow,dom}} from 'rsui/web'; import {{useState}} from 'react';
        import rsui from 'rsui'; import './app.css';
        function check(ok,why) {{if(!ok) throw new Error(why)}}
        function close(a,b,why) {{check(Math.abs(a-b)<0.01,why+': '+a+' vs '+b)}}
        function queryLog(run,forbid=false) {{
          const host=rsui.__host(),offset=host.scrollOffsetsOf,full=host.scrollOf,chains=host.chainsNow;
          const keys=new Set();let nonce=0;
          host.chainsNow=()=>1000000+(++nonce);
          host.scrollOffsetsOf=key=>{{keys.add(key);if(forbid)throw new Error('unused offset query '+key);return offset(key)}};
          host.scrollOf=key=>{{keys.add(key);if(forbid)throw new Error('unused full query '+key);return full(key)}};
          try {{return {{value:run(),keys}}}} finally {{host.scrollOffsetsOf=offset;host.scrollOf=full;host.chainsNow=chains}}
        }}
        {body}
    "#)).unwrap();
    hands::Hands::open(&file, (500,350))
}

#[test]
fn stable_rectangles_match_native_layout_without_offset_reads_after_wheel_scroll() {
    let mut ui = open("stable", r#"
        export default function App(){const [stage,setStage]=useState(0);return <PageWindow>
          <dom.div style={{width:1500,height:1600}}>
            <dom.div id="target" style={{marginLeft:40,marginTop:50,width:200,height:100}}>target</dom.div>
          </dom.div>
          <dom.button key="verify" style={{position:'fixed',top:300,left:10}} onClick={()=>{
            const target=document.getElementById('target'),host=rsui.__host();
            const chain=host.chainOf(target.__domKey),native=chain[chain.length-1];
            if(stage===1)check(window.scrollY>0,'native wheel moved the page');
            const result=queryLog(()=>target.getBoundingClientRect(),true);
            close(result.value.left,native[6],'native left');close(result.value.top,native[7],'native top');
            close(result.value.width,native[8],'native width');close(result.value.height,native[9],'native height');
            check(result.keys.size===0,'no scroll adjustment query without guesses');
            setStage(stage+1);
          }}>verify</dom.button><dom.p style={{position:'fixed',top:270}}>{'stage '+stage}</dom.p>
        </PageWindow>}
    "#);
    ui.click("verify");
    ui.wait_for("stage 1");
    ui.app.scroll(450.0, 100.0, 3).unwrap();
    ui.settle();
    ui.click("verify");
    ui.wait_for("stage 2");
}

#[test]
fn pending_root_and_nested_both_axis_requests_keep_native_reconciliation() {
    let mut ui = open("pending", r#"
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow>
          <dom.div style={{width:1500,height:1600}}>
            <dom.div id="nested" style={{width:200,height:90,overflow:'scroll'}}>
              <dom.div id="target" style={{width:800,height:600}}>target</dom.div>
            </dom.div>
          </dom.div><dom.button key="verify" style={{position:'fixed',top:300,left:10}} onClick={()=>{
            const nested=document.getElementById('nested'),target=document.getElementById('target');
            const before=target.getBoundingClientRect();window.scrollTo(31,120);nested.scrollTo(23,77);
            const {value:after,keys}=queryLog(()=>target.getBoundingClientRect());
            check(keys.has(nested.__domKey)&&keys.has('__dompage'),'both pending owners queried');
            close(before.left-after.left,54,'combined pending horizontal');
            close(before.top-after.top,197,'combined pending vertical');
            const fresh=target.getBoundingClientRect();
            close(fresh.left,after.left,'fresh horizontal');close(fresh.top,after.top,'fresh vertical');
            setSaid('pending passed');
          }}>verify</dom.button><dom.p style={{position:'fixed',top:270}}>{said}</dom.p>
        </PageWindow>}
    "#);
    ui.click("verify");
    ui.wait_for("pending passed");
}

#[test]
fn whole_offset_reads_keep_queries_and_positions_after_native_scroll() {
    let mut ui = open("whole", r#"
        let before;
        export default function App(){const [stage,setStage]=useState(0);return <PageWindow>
          <dom.div style={{width:1500,height:1600}}>
            <dom.div id="target" style={{position:'absolute',left:40,top:50,width:200,height:100}}>target</dom.div>
          </dom.div><dom.button key="verify" style={{position:'fixed',top:300,left:10}} onClick={()=>{
            const target=document.getElementById('target');
            const {value:offsets,keys}=queryLog(()=>[target.offsetLeft,target.offsetTop]);
            check(keys.has('__dompage'),'whole offsets query native page displacement');
            if(stage===0){before=offsets;close(offsets[0],40,'positioned initial offsetLeft');close(offsets[1],50,'positioned initial offsetTop')}else{
              close(offsets[0],before[0],'offsetLeft stable under native scroll');
              close(offsets[1],before[1],'offsetTop stable under native scroll');
            }
            setStage(stage+1);
          }}>verify</dom.button><dom.p style={{position:'fixed',top:270}}>{'stage '+stage}</dom.p>
        </PageWindow>}
    "#);
    ui.click("verify");
    ui.wait_for("stage 1");
    ui.app.scroll(450.0, 100.0, 3).unwrap();
    ui.app.scroll_across(450.0, 100.0, 3).unwrap();
    ui.settle();
    ui.click("verify");
    ui.wait_for("stage 2");
}

#[test]
fn fixed_boundary_keeps_nested_guess_and_excludes_pending_page_scroll() {
    let mut ui = open("fixed", r#"
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow>
          <dom.div style={{width:1500,height:1600}}>page</dom.div>
          <dom.div style={{position:'fixed',left:100,top:50,width:200,height:100}}>
            <dom.div id="nested" style={{width:80,height:50,overflow:'scroll'}}>
              <dom.div id="target" style={{width:800,height:600}}>target</dom.div>
            </dom.div>
          </dom.div><dom.button key="verify" style={{position:'fixed',top:300,left:10}} onClick={()=>{
            const nested=document.getElementById('nested'),target=document.getElementById('target');
            const before=target.getBoundingClientRect();window.scrollTo(31,120);nested.scrollTo(23,77);
            const {value:after,keys}=queryLog(()=>target.getBoundingClientRect());
            check(keys.has(nested.__domKey),'nested pending adjustment read');
            check(!keys.has('__dompage'),'fixed ancestor excludes page request');
            close(before.left-after.left,23,'only nested horizontal request');
            close(before.top-after.top,77,'only nested vertical request');
            setSaid('fixed passed');
          }}>verify</dom.button><dom.p style={{position:'fixed',top:270}}>{said}</dom.p>
        </PageWindow>}
    "#);
    ui.click("verify");
    ui.wait_for("fixed passed");
}
