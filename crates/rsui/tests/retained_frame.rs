#![cfg(all(
    feature = "retained-frame",
    feature = "render-diagnostics",
    not(feature = "automatic-render")
))]

use rsui::{
    App,
    core::{ActionKind, Element, Role},
    render::{Color, Renderer},
    runtime::Cx,
};

fn page(cx: &mut Cx, color: &Color) -> Element {
    let on = cx.state(|| false);
    let color = if cx.get(on) {
        Color::hex(0x33aa55)
    } else {
        *color
    };
    cx.on(
        Element::new(Role::Window)
            .key("root")
            .prop("background", color.to_prop())
            .child(Element::new(Role::Text).name("retained commands")),
        ActionKind::Toggle,
        move |cx, _| cx.update(on, |on| *on = !*on),
    )
}

fn app(color: Color) -> App {
    let mut app = App::new(
        page,
        color,
        rsui::default_text_engine().unwrap(),
        (160, 80),
        1.0,
    );
    app.set_animations(false);
    app.update();
    app
}

#[test]
fn unchanged_frames_reuse_the_picture_without_painting_or_copying_commands() {
    for flipping in [false, true] {
        let mut app = app(Color::hex(0x553399));
        let mut renderer = if flipping {
            Renderer::offscreen_flipping((160, 80))
        } else {
            Renderer::offscreen((160, 80))
        }
        .unwrap();
        app.render(&mut renderer).unwrap();
        let pixels = renderer.read_pixels().unwrap();
        let paint = app.scene_work();
        let storage = app.render_storage();
        let started = std::time::Instant::now();
        for _ in 0..100 {
            app.render(&mut renderer).unwrap();
            assert_eq!(
                app.scene_work(),
                paint,
                "no Painter traversal or command copies"
            );
        }
        eprintln!(
            "100 retained frames: {:?}, flipping={flipping}",
            started.elapsed()
        );
        assert_eq!(
            app.render_storage(),
            storage,
            "no additional retained command storage"
        );
        assert_eq!(renderer.read_pixels().unwrap(), pixels);
        app.runtime_mut()
            .dispatch(&[], ActionKind::Toggle, None)
            .unwrap();
        app.update();
        app.render(&mut renderer).unwrap();
        assert!(app.scene_work().serial > paint.serial);
        assert_ne!(renderer.read_pixels().unwrap(), pixels);
    }
}

#[test]
fn external_draws_scene_exports_and_renderer_resets_force_rebuilding() {
    let mut first = app(Color::hex(0x553399));
    let mut other = app(Color::hex(0xaa3311));
    let mut renderer = Renderer::offscreen((160, 80)).unwrap();
    first.render(&mut renderer).unwrap();
    let pixels = renderer.read_pixels().unwrap();
    other.render(&mut renderer).unwrap();
    first.render(&mut renderer).unwrap();
    assert_eq!(renderer.read_pixels().unwrap(), pixels);
    let paint = first.scene_work().serial;
    first.scene().quads.clear();
    first.render(&mut renderer).unwrap();
    assert!(first.scene_work().serial > paint);
    assert_eq!(renderer.read_pixels().unwrap(), pixels);
    let paint = first.scene_work().serial;
    renderer.trim_storage();
    first.render(&mut renderer).unwrap();
    assert!(first.scene_work().serial > paint);
    assert_eq!(renderer.read_pixels().unwrap(), pixels);
    let mut replacement = Renderer::offscreen((160, 80)).unwrap();
    first.renderer_replaced();
    first.render(&mut replacement).unwrap();
    assert!(first.scene_work().paint_passes > 0);
    assert_eq!(replacement.read_pixels().unwrap(), pixels);
}
