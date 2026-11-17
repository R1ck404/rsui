//! Host text conversion must preserve the generic child walker and observable reads.
mod hands;
use hands::Hands;
use std::path::PathBuf;

fn open(label: &str, body: &str) -> Hands {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("bridge-text-{}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("app.tsx");
    std::fs::write(&file, body).unwrap();
    Hands::open(&file, (400, 300))
}

#[test]
fn mutable_descriptions_getters_and_nested_children_keep_their_order() {
    let mut ui = open(
        "reads",
        r#"
        import { useState } from 'rsui';
        let reads = [];
        const props = { get color() {
            reads.push('color');
            if (kids[0] === 'first') kids[0] = 'read';
            return '#123456';
        } };
        const kids = ['first'];
        const label = new Proxy({
            get type() { reads.push('type'); return 'text'; },
            get key() { reads.push('key'); return 'label'; },
            get props() { reads.push('props'); return props; },
            get kids() { reads.push('kids'); return kids; }
        }, { get(target, key) { return Reflect.get(target, key); } });
        export default function App() {
            const [n, set] = useState(0);
            return <window>
                <button key="change" onClick={() => {
                    if (reads.join(',') !== 'type,key,props,color,kids') throw Error(reads.join(','));
                    reads = [];
                    kids[0] = n === 0 ? ['next', null, false, 7, <></>] : 'last';
                    set(n + 1);
                }}>Change</button>
                {label}
            </window>;
        }
    "#,
    );
    assert!(ui.says("read"));
    ui.click("change");
    assert!(ui.says("next7"), "{}", ui.app.outline());
    ui.click("change");
    assert!(ui.says("last"), "{}", ui.app.outline());
    rsui_tsx::census();
    ui.settle();
    assert!(ui.says("last"));
}

#[test]
fn labels_match_a_fresh_generic_tree_in_both_buffer_modes() {
    for flipping in [false, true] {
        let source = |generic: bool, initial: usize| {
            format!(
                r##"
            import {{ useState }} from 'rsui';
            export default function App() {{
                const [n, set] = useState({initial});
                const label = n ? 'Updated Ω 😀' : 'Initial';
                return <window background="#ffffff">
                    <button key="change" onClick={{() => set(1)}}>Change</button>
                    <text name="accessible" color="#123456" fontSize={{18}}>{{{child}}}</text>
                    <text>{{n ? '' : 'gone'}}</text>
                    <text>{{n ? 12 : -0}}</text>
                </window>;
            }}
        "##,
                child = if generic { "[[label]]" } else { "label" }
            )
        };
        let render = |ui: &mut Hands, renderer: &mut rsui::render::Renderer| {
            ui.settle();
            let scene = ui.app.scene();
            renderer.render(scene, rsui::render::Color::WHITE).unwrap();
            renderer.read_pixels().unwrap()
        };
        let renderer = || {
            if flipping {
                rsui::render::Renderer::offscreen_flipping((400, 300))
            } else {
                rsui::render::Renderer::offscreen((400, 300))
            }
            .unwrap()
        };
        let (pixels, outline, rects) = {
            let mut ui = open("updated", &source(false, 0));
            let mut gpu = renderer();
            render(&mut ui, &mut gpu);
            ui.click("change");
            rsui_tsx::census();
            (
                render(&mut ui, &mut gpu),
                ui.app.outline(),
                ui.app.rectangles(),
            )
        };
        let mut fresh = open("fresh", &source(true, 1));
        // Match hover/pressed state to the updated lane without changing its final state.
        fresh.click("change");
        assert_eq!(pixels, render(&mut fresh, &mut renderer()));
        assert_eq!(outline, fresh.app.outline());
        assert_eq!(rects, fresh.app.rectangles());
    }
}

#[test]
fn text_and_geometry_follow_actual_animation_frames() {
    let mut ui = open(
        "motion",
        r#"
        import { useState, useAnimated } from 'rsui';
        export default function App() {
            const [open, set] = useState(false);
            const width = useAnimated(open ? 180 : 60, 120);
            return <window>
                <button key="go" onClick={() => set(true)}>Go</button>
                <text key="moving" width={width}>{'Width ' + Math.round(width)}</text>
            </window>;
        }
    "#,
    );
    ui.app.set_animations(true);
    let now = std::time::Instant::now();
    ui.app.tick(now);
    ui.click("go");
    let mut widths = Vec::new();
    for step in 1..=8 {
        ui.app
            .tick(now + std::time::Duration::from_millis(step * 16));
        ui.settle();
        widths.push(ui.rect_of("moving")[2]);
    }
    assert!(
        widths.iter().any(|&width| width > 60. && width < 180.),
        "{widths:?}"
    );
    assert_eq!(widths.last(), Some(&180.));
    assert!(ui.says("Width 180"));
}
