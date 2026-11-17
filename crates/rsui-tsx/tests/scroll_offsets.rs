//! Offset queries preserve CSSOM semantics without measuring unused extents.
mod hands;

fn open(name: &str, body: &str) -> hands::Hands {
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-scroll-offsets-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("app.css"), "body {margin:0}").unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, format!(r#"
        import {{PageWindow,dom}} from 'rsui/web';
        import {{useState}} from 'react'; import rsui from 'rsui'; import './app.css';
        function check(ok,why) {{if(!ok) throw new Error(why)}}
        {body}
    "#)).unwrap();
    hands::Hands::open(&file, (500,350))
}

#[test]
fn both_axis_pending_requests_and_rects_only_request_offsets() {
    let mut ui = open("pending", r#"
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow>
        <dom.div style={{width:1500,height:1600}}>
          <dom.div id="nested" style={{width:200,height:90,overflow:'scroll'}}>
            <dom.div id="child" style={{width:800,height:600}}>child</dom.div>
          </dom.div>
          <dom.button key="verify" style={{position:'fixed',top:150,left:10}} onClick={()=>{
            const nested=document.getElementById('nested'),child=document.getElementById('child');
            const before=child.getBoundingClientRect();
            window.scrollTo(31,120);nested.scrollTo(23,77);
            check(nested.scrollLeft===23,'pending nested horizontal');
            const host=rsui.__host(),full=host.scrollOf,offset=host.scrollOffsetsOf,chains=host.chainsNow;
            let count=0,nonce=0;
            host.chainsNow=()=>1000000+(++nonce);
            host.scrollOf=()=>{throw new Error('unused full scroll extents requested')};
            host.scrollOffsetsOf=(key)=>{count++;return offset(key)};
            try {
              check(window.scrollX===31 && window.scrollY===120,'pending page both axes');
              check(window.pageXOffset===31 && window.pageYOffset===120,'page aliases');
              check(document.documentElement.scrollLeft===31 && document.documentElement.scrollTop===120,'document offsets');
              check(nested.scrollTop===77,'pending nested vertical');
              const after=child.getBoundingClientRect();
              check(Math.abs(before.left-after.left-54)<0.01,'nested and page horizontal rect offset');
              check(Math.abs(before.top-after.top-197)<0.01,'nested and page vertical rect offset');
              check(count>0,'offset path exercised');
            } finally {host.scrollOf=full;host.scrollOffsetsOf=offset;host.chainsNow=chains}
            setSaid('offsets passed');
          }}>verify</dom.button><dom.p style={{position:'fixed',top:250}}>{said}</dom.p>
        </dom.div></PageWindow>}
    "#);
    ui.click("verify");
    ui.wait_for("offsets passed");
}

#[test]
fn one_axis_native_change_discards_the_entire_pending_guess() {
    let mut ui = open("reconcile", r#"
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow>
        <dom.div id="nested" style={{width:200,height:90,overflow:'scroll'}}>
          <dom.div style={{width:800,height:600}}>child</dom.div>
        </dom.div><dom.button key="verify" onClick={()=>{
          const el=document.getElementById('nested'),live=el.__live,host=rsui.__host();
          const [x,y]=host.scrollOffsetsOf(el.__domKey);
          live.scrollGuess={fromX:x,fromY:y,x:23,y:77};
          check(el.scrollTop===77,'unchanged native pair retains pending guess');
          live.scrollGuess={fromX:x+1,fromY:y,x:23,y:77};
          check(el.scrollTop===y && live.scrollGuess===undefined,'horizontal native change clears both guesses');
          live.scrollGuess={fromX:x,fromY:y+1,x:23,y:77};
          check(el.scrollTop===y && live.scrollGuess===undefined,'vertical native change clears both guesses');
          setSaid('reconciliation passed');
        }}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#);
    ui.click("verify");
    ui.wait_for("reconciliation passed");
}

#[test]
fn native_offsets_follow_latest_structure_and_non_scrollers() {
    let mut ui = open("latest", r#"
        export default function App(){const [stage,setStage]=useState(0);return <PageWindow>
        <dom.div id="box" style={{width:200,height:90,overflow:stage===1?'scroll':'visible'}}>
          {stage===1?<dom.div style={{width:800,height:600}}>child</dom.div>:'box'}
        </dom.div><dom.button key="verify" onClick={()=>{
          const host=rsui.__host(),box=document.getElementById('box');
          host.keepLaidOut();
          const offsets=host.scrollOffsetsOf(box.__domKey),full=host.scrollOf(box.__domKey);
          check(host.scrollOffsetsOf('missing key')===undefined,'missing key');
          if(stage===1){
            check(offsets!==undefined && full!==undefined,'latest structure creates scroller');
            check(offsets[0]===full[0] && offsets[1]===full[1],'latest layout equivalence');
          } else {
            check(offsets===undefined && full===undefined,'latest non-scroller equivalence');
          }
          setStage(stage+1);
        }}>verify</dom.button><dom.p>{'stage '+stage}</dom.p></PageWindow>}
    "#);
    for stage in 1..=3 {
        ui.click("verify");
        ui.wait_for(&format!("stage {stage}"));
    }
}
