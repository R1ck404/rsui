//! Scroll observers hear positions that were requested, once per target per turn.

mod hands;

use std::sync::atomic::{AtomicU32, Ordering};

fn open(source: &str) -> hands::Hands {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "tsx-scroll-delivery-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(folder.join("app.css"), "body { margin: 0; }\n").unwrap();
    std::fs::write(&file, format!("import './app.css';\n{source}")).unwrap();
    hands::Hands::open(&file, (500, 350))
}

#[test]
fn repeated_scroll_requests_in_one_turn_are_coalesced_per_target() {
    let mut ui = open(
        r#"
        import { PageWindow, dom } from 'rsui/web';
        import { useState } from 'react';
        let rootEvents = [], innerEvents = [];
        window.addEventListener('scroll', () => rootEvents.push(window.scrollY));
        export default function App() {
            const [said, setSaid] = useState('');
            return <PageWindow>
                <dom.button key="request" id="request" onClick={() => {
                    const inner = document.getElementById('inner');
                    window.scrollTo(0, 100);
                    inner.scrollTop = 40;
                    window.scrollTo(0, 200);
                    inner.scrollTop = 80;
                    window.scrollTo(0, 200);
                    inner.scrollTop = 80;
                    queueMicrotask(() => setSaid(rootEvents.join(',') + '|' + innerEvents.join(',')));
                }}>request</dom.button>
                <dom.div id="inner" style={{height: 100, overflow: 'auto'}}
                     onScroll={event => innerEvents.push(event.currentTarget.scrollTop)}>
                    <dom.div style={{height: 1000}}>inner content</dom.div>
                </dom.div>
                <dom.div style={{height: 2000}}>page content</dom.div>
                <dom.p style={{position: 'fixed', top: 0}}>{'heard ' + said}</dom.p>
            </PageWindow>;
        }
        "#,
    );
    ui.click("request");
    ui.wait_for("heard 200|80");
    assert!(!ui.app.outline().contains("heard 200,200"));
}

#[test]
fn scrollend_listener_added_after_an_unobserved_scroll_still_hears_the_end() {
    let mut ui = open(
        r#"
        import { PageWindow, dom } from 'rsui/web';
        import { useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('waiting');
            return <PageWindow>
                <dom.button key="request" id="request" onClick={() => {
                    window.scrollTo(0, 200);
                    // The scroll notification was already queued. This listener
                    // is installed afterwards, but before scrolling ends.
                    queueMicrotask(() => document.addEventListener('scrollend',
                        () => setSaid('late end ' + window.scrollY), {once: true}));
                }}>request</dom.button>
                <dom.div style={{height: 2000}}>page content</dom.div>
                <dom.p style={{position: 'fixed', top: 0}}>{said}</dom.p>
            </PageWindow>;
        }
        "#,
    );
    ui.click("request");
    ui.wait_for("late end 200");
}

#[test]
fn walking_measured_ancestors_reuses_the_current_chain_and_refreshes_after_scrolling() {
    let mut ui = open(
        r#"
        import { PageWindow, dom } from 'rsui/web';
        import { useState } from 'react';
        import * as rsui from 'rsui';
        let initialTop = null;
        export default function App() {
            const [said, setSaid] = useState('waiting');
            const measure = () => {
                const leaf = document.getElementById('leaf');
                const host = rsui.__host(), original = host.chainOf;
                let reads = 0, top, names = [], phase, keys = [];
                host.chainOf = key => { reads++; keys.push(key); return original(key); };
                try {
                    top = leaf.getBoundingClientRect().top;
                    for (let at = leaf.parentElement; at !== document.body; at = at.parentElement) {
                        at.getBoundingClientRect();
                        if (at.id) names.push(at.id);
                        if (at.id === 'inner') phase = at.dataset.phase;
                    }
                } finally {
                    host.chainOf = original;
                }
                if (initialTop === null) initialTop = top;
                setSaid('walk ' + (reads <= 1 ? 'bounded' : 'unbounded ' + reads + ' keys ' + keys.join(','))
                    + ' ' + names.join(',') + ' moved ' + Math.round(initialTop - top) + ' phase ' + phase);
            };
            return <PageWindow>
                <dom.div style={{position: 'fixed', top: 0, left: 300}}>
                    <dom.button key="measure" onClick={measure}>measure</dom.button>
                    <dom.button key="scroll-trigger" onClick={() => {
                        document.getElementById('inner').setAttribute('data-phase', 'updated');
                        window.scrollTo(0, 300);
                        setTimeout(measure, 0);
                    }}>scroll</dom.button>
                    <dom.p>{said}</dom.p>
                </dom.div>
                <dom.div id="outer" style={{height: 2000}}>
                    <dom.div id="middle" style={{paddingTop: 400}}>
                        <dom.div id="inner" data-phase="initial" style={{paddingTop: 100}}>
                            <dom.div id="leaf" style={{height: 20}}>leaf</dom.div>
                        </dom.div>
                    </dom.div>
                </dom.div>
            </PageWindow>;
        }
        "#,
    );
    ui.click("measure");
    ui.wait_for("walk bounded inner,middle,outer moved 0 phase initial");
    ui.click("scroll-trigger");
    ui.wait_for("walk bounded inner,middle,outer moved 300 phase updated");
}

#[test]
fn page_computed_style_methods_use_their_receiver_and_keep_each_result_isolated() {
    let mut ui = open(
        r#"
        import { PageWindow, dom } from 'rsui/web';
        import { useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('waiting');
            return <PageWindow>
                <dom.button key="inspect-style" onClick={() => {
                    const first = getComputedStyle(document.body);
                    const second = getComputedStyle(document.documentElement);
                    first.color = 'first';
                    second.color = 'second';
                    const get = first.getPropertyValue;
                    const item = first.item;
                    const priority = first.getPropertyPriority;
                    const oldOverflow = document.body.style.overflow;
                    const oldPointerEvents = document.body.style.pointerEvents;
                    document.body.style.overflow = 'hidden';
                    document.body.style.pointerEvents = 'none';
                    const modified = getComputedStyle(document.body);
                    const modifiedRoot = getComputedStyle(document.documentElement);
                    if (oldOverflow === undefined) delete document.body.style.overflow;
                    else document.body.style.overflow = oldOverflow;
                    document.body.style.pointerEvents = oldPointerEvents;
                    const restored = getComputedStyle(document.body);
                    const names = Object.keys(first);
                    setSaid([
                        first !== second,
                        get.call(first, 'color'),
                        get.call(second, 'color'),
                        item.call(first, 0) === Object.keys(first)[0],
                        item.call(second, 0) === Object.keys(second)[0],
                        priority.call(second, 'color') === '',
                        getComputedStyle(document.body).color !== 'first',
                        getComputedStyle(document.documentElement).color !== 'second',
                        first.width + 'x' + first.height,
                        modified.overflow === 'hidden' && modified.pointerEvents === 'none'
                            && modifiedRoot.overflow === 'hidden' && modifiedRoot.pointerEvents === 'none',
                        restored.overflow === first.overflow && restored.pointerEvents === first.pointerEvents,
                        names.indexOf('width') === names.indexOf('boxSizing') + 1
                            && names.indexOf('height') === names.indexOf('width') + 1,
                    ].join('|'));
                }}>inspect</dom.button>
                <dom.p>{said}</dom.p>
            </PageWindow>;
        }
        "#,
    );
    // A native window announces its initial size through resize too; Hands
    // constructs the app directly and needs that first notification explicitly.
    ui.app.resize((500, 350), 1.0);
    ui.settle();
    ui.click("inspect-style");
    ui.wait_for("true|first|second|true|true|true|true|true|500pxx350px|true|true|true");
    ui.app.resize((640, 420), 1.0);
    ui.settle();
    ui.click("inspect-style");
    ui.wait_for("true|first|second|true|true|true|true|true|640pxx420px|true|true|true");
}

#[test]
fn measured_geometry_preserves_nested_transforms_pending_scrolls_and_fixed_boundaries() {
    let mut ui = open(
        r#"
        import { PageWindow, dom } from 'rsui/web';
        import { useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('waiting');
            const rect = id => {
                const r = document.getElementById(id).getBoundingClientRect();
                return [r.left, r.top, r.width, r.height].map(n => Math.round(n * 1000) / 1000).join(',');
            };
            return <PageWindow>
                <dom.div style={{position: 'fixed', top: 300, left: 0}}>
                    <dom.button key="inspect-geometry" onClick={() => {
                        const seen = [rect('transformed-target'), rect('projected-target')];
                        // These reads happen in the same event, before native
                        // rendering applies either CSSOM write.
                        document.getElementById('transformed-target').style.transform = 'translate(25%, 50%)';
                        seen.push(rect('transformed-target'));
                        document.getElementById('transform-parent').style.transformOrigin = '50% 50%';
                        seen.push(rect('transformed-target'));
                        window.scrollTo(0, 100);
                        document.getElementById('nested-scroller').scrollTop = 50;
                        seen.push(rect('transformed-target'), rect('plain-target'), rect('fixed-target'), rect('nested-fixed'));
                        const first = document.getElementById('plain-target').getBoundingClientRect();
                        const second = document.getElementById('fixed-target').getBoundingClientRect();
                        const serialize = first.toJSON;
                        first.x = 123;
                        first.left = 124;
                        const snapshot = serialize.call(first);
                        snapshot.x = -1;
                        seen.push([
                            first !== second,
                            serialize.call(first).x === 123,
                            serialize.call(first).left === 124,
                            serialize.call(second).x === second.x,
                            serialize.call(second).y === second.y,
                            serialize.call(first) !== serialize.call(first),
                            Object.keys(first).join(',') === 'x,y,left,top,right,bottom,width,height,toJSON',
                            Object.keys(snapshot).join(',') === 'x,y,width,height,top,right,bottom,left',
                            second.x === 250,
                        ].join(','));
                        setSaid(seen.join('|'));
                    }}>inspect geometry</dom.button>
                    <dom.p>{said}</dom.p>
                </dom.div>
                <dom.div style={{position: 'relative', width: 300, height: 200}}>
                    <dom.div id="transform-parent" style={{position: 'absolute', left: 20, top: 40, width: 200, height: 100,
                        transform: 'translate(30px, 20px) scale(2, 1)', transformOrigin: '0% 0%'}}>
                        <dom.div id="transformed-target" style={{position: 'absolute', left: 10, top: 5, width: 100, height: 40,
                            transform: 'translate(50%, 25%)', transformOrigin: '0 0'}}>transformed</dom.div>
                    </dom.div>
                    <dom.div id="projected-target" style={{position: 'absolute', left: 50, top: 60, width: 100, height: 40,
                        transform: 'matrix3d(1, 0, 0, 0.001, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)', transformOrigin: '0 0'}}>projected</dom.div>
                </dom.div>
                <dom.div id="nested-scroller" style={{position: 'absolute', left: 300, top: 200, width: 150, height: 100, overflow: 'auto'}}>
                    <dom.div style={{height: 600}}>
                        <dom.div id="plain-target" style={{width: 40, height: 20}}>plain</dom.div>
                    </dom.div>
                    <dom.div id="nested-fixed" style={{position: 'fixed', left: 260, top: 110, width: 25, height: 15}}>nested fixed</dom.div>
                </dom.div>
                <dom.div id="fixed-target" style={{position: 'fixed', left: 250, top: 100, width: 30, height: 20}}>fixed</dom.div>
                <dom.div style={{height: 2000}}>page content</dom.div>
            </PageWindow>;
        }
        "#,
    );
    ui.click("inspect-geometry");
    ui.wait_for(
        "170,75,200,40|50,60,90.909,40|120,85,200,40|20,85,200,40|20,-15,200,40|300,50,40,20|250,100,30,20|260,110,25,15|true,true,true,true,true,true,true,true,true",
    );
}

#[test]
fn translation_only_chains_compose_with_matrix_fallbacks_in_the_correct_order() {
    let mut ui = open(
        r#"
        import { PageWindow, dom } from 'rsui/web';
        import { useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('waiting');
            return <PageWindow>
                <dom.button key="inspect-translations" style={{position: 'fixed', left: 0, top: 0}} onClick={() => {
                    setSaid(['translated', 'scale-then-translate', 'translate-then-scale', 'calculated', 'matrix3d-translation', 'nonzero-z', 'rotated'].map(id => {
                        const r = document.getElementById(id).getBoundingClientRect();
                        return [r.left, r.top, r.width, r.height].map(n => Math.round(n * 1000) / 1000).join(',');
                    }).join('|'));
                }}>inspect translations</dom.button>
                <dom.div style={{position: 'relative', width: 500, height: 320}}>
                    <dom.div style={{position: 'absolute', left: 20, top: 20, width: 200, height: 200,
                        transform: 'translateX(10px) translateY(5px)', transformOrigin: '100% 0%'}}>
                        <dom.div style={{position: 'absolute', left: 10, top: 10, width: 80, height: 40,
                            transform: 'matrix(1, 0, 0, 1, 7, -3)', transformOrigin: '80% 20%'}}>
                            <dom.div id="translated" style={{position: 'absolute', left: 5, top: 5, width: 20, height: 10,
                                transform: 'translate3d(50%, 100%, 0px)', transformOrigin: '33% 66%'}}>translated</dom.div>
                        </dom.div>
                    </dom.div>
                    <dom.div style={{position: 'absolute', left: 250, top: 20, width: 100, height: 100,
                        transform: 'translate3d(12px, -4px, 0)', transformOrigin: '75% 25%'}}>
                        <dom.div id="scale-then-translate" style={{position: 'absolute', left: 10, top: 5, width: 20, height: 10,
                            transform: 'scale(2, 3)', transformOrigin: '0 0'}}>scaled</dom.div>
                    </dom.div>
                    <dom.div style={{position: 'absolute', left: 20, top: 180, width: 100, height: 100,
                        transform: 'scale(2, 1)', transformOrigin: '0 0'}}>
                        <dom.div id="translate-then-scale" style={{position: 'absolute', left: 10, top: 5, width: 20, height: 10,
                            transform: 'translateX(50%) translateY(20%)', transformOrigin: '100% 100%'}}>translated first</dom.div>
                    </dom.div>
                    <dom.div id="calculated" style={{position: 'absolute', left: 250, top: 180, width: 20, height: 10,
                        transform: 'translate(calc(50% + 3px), calc(100% - 2px))', transformOrigin: '100% 100%'}}>calc</dom.div>
                    <dom.div id="matrix3d-translation" style={{position: 'absolute', left: 350, top: 180, width: 20, height: 10,
                        transform: 'matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -5, 7, 0, 1)', transformOrigin: '10% 80%'}}>matrix3d</dom.div>
                    <dom.div id="nonzero-z" style={{position: 'absolute', left: 400, top: 180, width: 20, height: 10,
                        transform: 'translate3d(3px, 4px, 9px)', transformOrigin: '50% 50%'}}>nonzero Z</dom.div>
                    <dom.div id="rotated" style={{position: 'absolute', left: 350, top: 250, width: 20, height: 10,
                        transform: 'rotate(90deg)', transformOrigin: '0 0'}}>rotated</dom.div>
                </dom.div>
                <dom.p style={{position: 'fixed', top: 280, left: 0}}>{said}</dom.p>
            </PageWindow>;
        }
        "#,
    );
    ui.click("inspect-translations");
    ui.wait_for("62,47,20,10|272,21,40,30|60,187,40,10|263,188,20,10|345,187,20,10|403,184,20,10|340,250,10,20");
}
