#![cfg(all(feature = "automatic-render", feature = "render-diagnostics"))]
use rsui::{App, render::Renderer};
const SOURCE: &str = r#"import {useState} from 'react';
function Target(){const [n,set]=useState(0);return <text key="target" onClick={()=>set(n+1)} height={30} width={n>=6?150:100} background={n%2?0xd03020ff:0x205080ff}>{n>=5?'Changed caf\u00e9 \u65e5\u672c':'Save'}</text>}
export default function App(){return <window><Target/><Target/></window>}"#;
fn lane(flip: bool) -> (App, Renderer, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "011d-oracle-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("app.tsx");
    std::fs::write(&file, SOURCE).unwrap();
    let mut app = App::new(
        rsui_tsx::root,
        rsui_tsx::load(&file, 32 * 1024 * 1024).unwrap(),
        rsui::default_text_engine().unwrap(),
        (180, 140),
        1.0,
    );
    app.set_animations(false);
    let gpu = if flip {
        Renderer::offscreen_flipping((180, 140))
    } else {
        Renderer::offscreen((180, 140))
    }
    .unwrap();
    (app, gpu, file)
}
#[test]
fn separate_vm_threads_match_full_reference_with_real_actions() {
    for flip in [false, true] {
        let (send, recv) = std::sync::mpsc::channel::<Option<f32>>();
        let (pixels_send, pixels_recv) = std::sync::mpsc::channel();
        let reference = std::thread::spawn(move || {
            let (mut app, mut gpu, file) = lane(flip);
            gpu.set_always_redraw(true);
            while let Ok(y) = recv.recv() {
                if let Some(y) = y {
                    app.click(10.0, y).unwrap();
                }
                app.update();
                let scene = app.scene_full_for_diagnostics();
                let clear = scene
                    .window_color
                    .unwrap_or(rsui::prelude::Color::hex(0xF6F8FA));
                gpu.render(scene, clear).unwrap();
                pixels_send.send(gpu.read_pixels().unwrap()).unwrap();
            }
            std::fs::remove_file(file).unwrap();
        });
        let (mut app, mut gpu, file) = lane(flip);
        let mut before = Vec::new();
        let mut fast = 0;
        for step in 0..16 {
            let y = if step == 0 {
                None
            } else {
                Some(if step > 7 && step % 2 == 0 {
                    45.0
                } else {
                    10.0
                })
            };
            send.send(y).unwrap();
            if let Some(y) = y {
                app.click(10.0, y).unwrap();
            }
            app.update();
            app.render(&mut gpu).unwrap();
            let pixels = gpu.read_pixels().unwrap();
            assert_eq!(pixels, pixels_recv.recv().unwrap(), "step {step}");
            assert_ne!(pixels, before, "inert step {step}");
            before = pixels;
            fast += usize::from(
                app.scene_work().path == rsui::render::diagnostics::ScenePath::OwnedPalette,
            );
        }
        assert!(fast >= 2);
        drop(send);
        reference.join().unwrap();
        std::fs::remove_file(file).unwrap();
    }
}
