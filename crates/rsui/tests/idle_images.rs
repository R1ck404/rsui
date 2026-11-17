//! Idle image retirement must reach the renderer without an extra frame.
#![cfg(feature = "render-diagnostics")]
use rsui::App;
use rsui::core::{ActionKind, Element, Role};
use rsui::render::{Renderer, diagnostics::DrawOutcome};
use rsui::runtime::Cx;
use std::time::Duration;

// Allows the exact regression to run on the baseline before the delivery API exists.
#[allow(dead_code)]
trait BaselineIdle {
    fn release_unused_images(&mut self, _: &mut Renderer) {}
}
impl BaselineIdle for App {}

fn gallery(cx: &mut Cx, _: &()) -> Element {
    let large = cx.state(|| true);
    let mut root = Element::new(Role::Window).key("root");
    for i in 0..if cx.get(large) { 12 } else { 1 } {
        root = root.child(
            Element::new(Role::Image)
                .prop("width", 40.0)
                .prop("height", 40.0)
                .prop("src", format!("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='40' height='40'><rect width='40' height='40' fill='rgb({},50,100)'/></svg>", i * 20)),
        );
    }
    cx.on(root, ActionKind::Toggle, move |cx, _| {
        cx.update(large, |v| *v = !*v)
    })
}

fn settle(app: &mut App, gpu: &mut Renderer) {
    for _ in 0..3 {
        app.render(gpu).unwrap();
        assert!(app.wait_for_images(Duration::from_secs(10)));
        app.update();
    }
    app.render(gpu).unwrap();
}

#[test]
fn unused_images_leave_gpu_at_idle_without_drawing_and_recover_on_revisit() {
    for flipping in [false, true] {
        let make = || {
            if flipping {
                Renderer::offscreen_flipping((100, 520))
            } else {
                Renderer::offscreen((100, 520))
            }
            .unwrap()
        };
        let mut app = App::new(
            gallery,
            (),
            rsui::default_text_engine().unwrap(),
            (100, 520),
            1.0,
        );
        app.set_animations(false);
        app.update();
        let mut gpu = make();
        settle(&mut app, &mut gpu);
        assert_eq!(gpu.retained_storage().image_entries, 12);
        let large = gpu.read_pixels().unwrap();
        app.runtime_mut()
            .dispatch(&[], ActionKind::Toggle, None)
            .unwrap();
        app.update();
        settle(&mut app, &mut gpu);
        let small = gpu.read_pixels().unwrap();
        let serial = gpu.draw_work().serial;
        assert!(!gpu.compact().unwrap());
        app.forget_caches();
        app.release_unused_images(&mut gpu);
        assert_eq!(gpu.draw_work().serial, serial, "cleanup must not draw");
        assert_eq!(
            gpu.retained_storage().image_entries,
            1,
            "idle must deliver retirements"
        );
        assert_eq!(gpu.read_pixels().unwrap(), small);
        app.render(&mut gpu).unwrap();
        assert_eq!(gpu.draw_work().outcome, DrawOutcome::Unchanged);
        assert_eq!(gpu.read_pixels().unwrap(), small);
        // Repeated cleanup is a no-op; device replacement still reloads the visible image.
        app.forget_caches();
        app.release_unused_images(&mut gpu);
        gpu = make();
        app.renderer_replaced();
        settle(&mut app, &mut gpu);
        assert_eq!(gpu.read_pixels().unwrap(), small);
        app.runtime_mut()
            .dispatch(&[], ActionKind::Toggle, None)
            .unwrap();
        app.update();
        settle(&mut app, &mut gpu);
        assert_eq!(gpu.retained_storage().image_entries, 12);
        assert_eq!(gpu.read_pixels().unwrap(), large);
    }
}
