use super::*;
use rsui_js::{Engine, HostFunction};

const PRELUDE: &str = r#"
function check(ok,why){if(!ok)throw new Error(why)}
function make(count=1,keyLength=1){
 const parent={key:undefined,parent:null,children:[],detached:false};
 const map=new Map(),lives=new Map(),hidden=new Map();let first;
 for(let index=0;index<count;index++){
  const key='a'.repeat(keyLength)+index;
  const live={given:{role:'checkbox',disabled:false},tag:'input',state:0};
  live.weak=new WeakRef(live);
  const node={key,live,parent,index,children:[],entry:[key,'group',undefined,undefined,undefined,'',0,0,0,0,undefined,key,false]};
  parent.children.push(node);map.set(key,node);lives.set(key,live.weak);first??={key,live,node};
 }
 return {parent,map,lives,hidden,...first};
}
var fields=['role','disabled'];
function take(f,names=fields,epoch=1){return capture(f.map,names,epoch,f.lives,f.hidden)}
function valid(r,f,names=fields,epoch=1){return kept(r,f.map,names,epoch,f.lives,f.hidden)}
"#;

fn engine() -> Engine {
    let mut engine = Engine::new();
    for (name, length, function) in [
        ("capture", 5, capture as HostFunction),
        ("kept", 6, kept as HostFunction),
        ("safePrograms", 1, safe_programs as HostFunction),
        ("safeIntrinsics", 0, safe_intrinsics as HostFunction),
    ] {
        let function = engine.vm().host_function(name, length, function);
        engine.vm().set_global(name, function);
    }
    engine.eval(PRELUDE, "selector-receipt-test.js").unwrap();
    engine
}

fn run(source: &str) {
    engine().eval(source, "selector-receipt-test.js").unwrap();
}

#[test]
fn primitives_merge_mutations_and_new_graphs_are_exact() {
    run(r#"
const f=make();const r=take(f);check(Array.isArray(r),'captured');check(valid(r,f),'same');
f.live.given.role='button';check(!valid(r,f),'in-place primitive mutation');
f.live.given.role='checkbox';check(valid(r,f),'restored input');
f.live.given.style={padding:1};check(valid(r,f),'unused object allowed');
f.live.given.style.padding=8;check(valid(r,f),'unused object mutation ignored');
f.live.attributes={role:undefined};check(!valid(r,f),'enumerable undefined overrides given');
f.live.attributes={};
Object.defineProperty(f.live.attributes,'role',{value:'ignored',enumerable:false});
check(valid(r,f),'nonenumerable attribute not spread');
const p={key:undefined,parent:null,children:[],detached:false};
const n={...f.node,parent:p};p.children=[n];const m=new Map([[f.key,n]]);
check(kept(r,m,fields,1,f.lives,f.hidden),'equivalent new graph');
check(!kept(r,m,['role'],1,f.lives,f.hidden),'field program changed');
check(!kept(r,m,fields,2,f.lives,f.hidden),'recipe epoch changed');
check(r.every(v=>v===null||typeof v!=='object'||v instanceof WeakRef),'only primitives or WeakRefs retained');
"#);
}

#[test]
fn authoritative_rosters_and_hidden_topology_are_checked() {
    run(r#"
const f=make();const r=take(f);
const replacement={given:{role:'checkbox',disabled:false},tag:'input',state:0};
const otherWeak=new WeakRef(replacement);f.lives.set(f.key,otherWeak);
check(!valid(r,f),'old graph cannot certify replaced authoritative owner');
f.lives.set(f.key,f.live.weak);check(valid(r,f),'original roster restored');
f.live.parentKey='some-parent';f.live.listPart=true;f.live.text='hidden';f.live.notShown=true;
f.hidden.set(f.key,f.live);check(!valid(r,f),'hidden roster added');
const hidden=take(f);check(Array.isArray(hidden)&&valid(hidden,f),'hidden captured');
f.live.parentKey='different-parent';check(!valid(hidden,f),'hidden parent changed');
f.live.parentKey='some-parent';f.live.listPart=false;check(!valid(hidden,f),'hidden list membership changed');
f.hidden.clear();check(valid(r,f),'restored empty hidden roster');
"#);
}

#[test]
fn ordinary_styled_bag_after_style_removal_can_be_certified() {
    run(r#"
const f=make();f.live.given.style={height:2000};delete f.live.given.style;
const r=take(f);check(Array.isArray(r)&&valid(r,f),'ordinary data dictionary after Styled deletes style');
f.live.given.role='button';check(!valid(r,f),'dictionary values are still compared exactly');
Object.defineProperty(f.live.given,'unrelatedGetter',{get(){throw new Error('must not execute')}});
check(take(f)===undefined,'dictionary accessor remains unsupported');
"#);
}

#[test]
fn getters_proxies_and_spread_prototypes_never_execute() {
    run(r#"
const f=make();let calls=0;
Object.defineProperty(f.live.given,'unused',{get(){calls++;return 1},configurable:true});
check(take(f)===undefined&&calls===0,'unused source getter declined without execution');
delete f.live.given.unused;
const proto={};Object.defineProperty(proto,'role',{get(){calls++;return 'checkbox'}});
f.live.given=Object.create(proto);f.live.given.disabled=false;
check(take(f)===undefined&&calls===0,'selected inherited getter declined');
f.live.attributes={};check(Array.isArray(take(f))&&calls===0,'spread ignores source prototype');
f.live.given=new Proxy({role:'checkbox'},{get(){calls++;return 1}});
check(take(f)===undefined&&calls===0,'proxy never read');
f.live.given={role:'checkbox',disabled:false};
const fieldsProxy=new Proxy(fields,{get(){calls++;return 'role'}});
check(take(f,fieldsProxy)===undefined&&calls===0,'fields proxy never read');
const inherited={disabled:true}, inheritedFixture=make();
const inheritedLive=Object.create({attributes:inherited});
inheritedLive.given={role:'checkbox',disabled:false};inheritedLive.tag='input';inheritedLive.state=0;
inheritedLive.weak=new WeakRef(inheritedLive);inheritedFixture.node.live=inheritedLive;
inheritedFixture.live=inheritedLive;inheritedFixture.lives.set(inheritedFixture.key,inheritedLive.weak);
const inheritedReceipt=take(inheritedFixture);check(Array.isArray(inheritedReceipt),'inherited data attributes captured');
inherited.disabled=false;check(!valid(inheritedReceipt,inheritedFixture),'inherited bag mutation seen');
"#);
}

#[test]
fn bounded_receipts_decline_huge_strings_and_over_budget_rows() {
    run(r#"
const f=make();check(take(f,['r'.repeat(1000000)])===undefined,'huge field rejected before copying');
f.live.given.role='x'.repeat(5000);check(take(f)===undefined,'oversized primitive rejected');
const large=make(1800,200);check(take(large)===undefined,'whole receipt exceeds one MiB');
check(take(make(),Array(65).fill('role'))===undefined,'field count bounded');
"#);
}

#[test]
fn receipt_does_not_retain_lives_or_property_bags_after_gc() {
    let mut engine = engine();
    engine
        .eval(
            r#"
globalThis.f=make();globalThis.r=take(f);check(Array.isArray(r),'receipt exists');
globalThis.liveWeak=new WeakRef(f.live);globalThis.givenWeak=new WeakRef(f.live.given);
globalThis.f=null;
"#,
            "selector-receipt-gc.js",
        )
        .unwrap();
    engine.vm().clear_kept_objects();
    engine.collect_garbage();
    engine.eval("check(liveWeak.deref()===undefined&&givenWeak.deref()===undefined,'receipt retains no live/props bag');check(Array.isArray(r),'receipt remains alive')", "selector-receipt-gc.js").unwrap();
}

#[test]
fn program_preflight_rejects_accessor_and_proxy_recipes_without_calls() {
    run(r#"
const f=make();let calls=0;
const step={tag:null,id:null,part:null,link:null,unknown:false,self:false,classes:[],attrs:[{name:'role',op:'=',value:'checkbox'}],pseudo:[]};
const one={selectors:[[step]],was:false};f.live.hasDrawn=new Map([['recipe',one]]);
const watched=new Set([f.live]);check(safePrograms(watched),'ordinary program');
Object.defineProperty(step,'attrs',{get(){calls++;return []},configurable:true});
check(!safePrograms(watched)&&calls===0,'getter AST rejected before classification');
Object.defineProperty(step,'attrs',{value:[],configurable:true});
one.selectors=new Proxy([],{get(){calls++;return []}});
check(!safePrograms(watched)&&calls===0,'proxy program declined');
"#);
}

#[test]
fn public_intrinsic_mutation_disables_capture_and_validation() {
    run(r#"
const f=make(),r=take(f);check(safeIntrinsics(),'canonical initially');
const original=String.prototype.includes;String.prototype.includes=function(){return false};
check(!safeIntrinsics()&&take(f)===undefined&&!valid(r,f),'modified matching intrinsic declined');
String.prototype.includes=original;check(safeIntrinsics()&&valid(r,f),'exact intrinsic restored');
"#);
}
