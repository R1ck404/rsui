//! Test-only independent WARP lanes. Resource IDs/atlas coordinates are deliberately
//! not compared: each GPU receives only its App's uploads, from its first frame.
use rsui::prelude::*;
use rsui::render::diagnostics::{DrawWork, ScenePath, SceneWork};
use rsui::render::{Renderer, Scene};
use rsui::text::TextEngine;
use rsui::{App, Frame};

pub struct Lane {
    pub app: App,
    pub renderer: Renderer,
    full: bool,
}

#[derive(Debug)]
pub struct Capture {
    pub pixels: Vec<u8>,
    pub scene: Scene,
    pub work: SceneWork,
    pub draw_before: DrawWork,
    pub draw: DrawWork,
    pub update: Frame,
    pub outline: String,
    pub rectangles: String,
    pub focus: Option<Vec<u32>>,
    pub selection: Option<String>,
    pub caret: Option<(f32, f32, f32)>,
}

impl Lane {
    pub fn new<P: 'static>(
        component: fn(&mut Cx, &P) -> Element,
        props: P,
        size: (u32, u32),
        scale: f32,
        flipping: bool,
        full: bool,
    ) -> Self {
        let mut app = App::new(
            component,
            props,
            TextEngine::new(crate::font::plain_font()),
            size,
            scale,
        );
        app.set_animations(false);
        let mut renderer = if flipping {
            Renderer::offscreen_flipping(size)
        } else {
            Renderer::offscreen(size)
        }
        .expect("WARP renderer");
        renderer.set_always_redraw(full);
        Self {
            app,
            renderer,
            full,
        }
    }

    pub fn resize(&mut self, size: (u32, u32), scale: f32) {
        self.app.resize(size, scale);
        self.renderer.resize(size).unwrap();
    }

    pub fn frame(&mut self) -> Capture {
        self.frame_with_full_paint(self.full)
    }

    /// Test an individual reference call after ordinary calls have populated the
    /// caret cache. No App mode is retained, and GPU redraw policy is independent.
    pub fn frame_with_full_paint(&mut self, full: bool) -> Capture {
        let update = self.app.update();
        let scene = if full {
            self.app.scene_full_for_diagnostics()
        } else {
            self.app.scene()
        };
        // Before draw drains resources and swaps the command lists.
        let snapshot = scene.clone();
        let draw_before = self.renderer.draw_work();
        let clear = scene.window_color.unwrap_or(Color::hex(0xF6F8FA));
        self.renderer.render(scene, clear).unwrap();
        // SceneWork is saved by Painter before swaps; it remains the prepared frame.
        let work = self.app.scene_work();
        let storage = self.app.render_storage();
        assert_eq!(storage.glyph_payload.len_bytes, 0);
        assert_eq!(storage.image_payload.len_bytes, 0);
        assert_eq!(snapshot.quads.len() as u64, work.output.quads);
        assert_eq!(snapshot.uploads.len() as u64, work.pending_glyphs.count);
        assert_eq!(
            snapshot.image_uploads.len() as u64,
            work.pending_images.count
        );
        assert_eq!(self.app.scene_work(), work);
        if full {
            assert_eq!(work.path, ScenePath::FullTraversal);
            assert!(work.paint_passes > 0 && work.node_enters > 0);
            assert_eq!(work.caret_copy_bytes, 0);
        } else if work.path == ScenePath::CaretReuse {
            assert_eq!((work.node_enters, work.paint_passes), (0, 0));
            assert!(work.caret_copy_bytes > 0);
        } else {
            assert!(work.node_enters > 0);
        }
        let draw = self.renderer.draw_work();
        assert_eq!(draw.input, work.output);
        assert_eq!(draw.serial, draw_before.serial + 1);
        assert_eq!(
            self.app.scene_work(),
            work,
            "saved record survives draw swaps"
        );
        Capture {
            pixels: self.renderer.read_pixels().unwrap(),
            scene: snapshot,
            work,
            draw_before,
            draw,
            update,
            outline: self.app.outline(),
            rectangles: self.app.rectangles(),
            focus: self.app.focused().map(<[u32]>::to_vec),
            selection: self.app.selected_text().map(str::to_owned),
            caret: self.app.caret_place(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub first: (u32, u32),
    pub candidate: [u8; 4],
    pub reference: [u8; 4],
    pub count: usize,
    /// Inclusive pixel bounds.
    pub bounds: (u32, u32, u32, u32),
}

pub fn difference(a: &[u8], b: &[u8], size: (u32, u32)) -> Option<Mismatch> {
    assert_eq!(a.len(), (size.0 * size.1 * 4) as usize);
    assert_eq!(a.len(), b.len());
    let mut result: Option<Mismatch> = None;
    for (i, (a, b)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        if a == b {
            continue;
        }
        let (x, y) = (i as u32 % size.0, i as u32 / size.0);
        let d = result.get_or_insert(Mismatch {
            first: (x, y),
            candidate: a.try_into().unwrap(),
            reference: b.try_into().unwrap(),
            count: 0,
            bounds: (x, y, x, y),
        });
        d.count += 1;
        d.bounds = (
            d.bounds.0.min(x),
            d.bounds.1.min(y),
            d.bounds.2.max(x),
            d.bounds.3.max(y),
        );
    }
    result
}

pub fn compare(
    label: &str,
    size: (u32, u32),
    scale: f32,
    flipping: bool,
    a: &Capture,
    b: &Capture,
) {
    assert_eq!(a.draw.serial, a.draw_before.serial + 1);
    assert_eq!(b.draw.serial, b.draw_before.serial + 1);
    let mismatch = difference(&a.pixels, &b.pixels, size);
    let semantic = a.outline == b.outline
        && a.rectangles == b.rectangles
        && a.focus == b.focus
        && a.selection == b.selection
        && a.caret == b.caret;
    if mismatch.is_some() || !semantic {
        let name: String = label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/003c-oracle")
            .join(format!("{name}-{flipping}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("candidate.rgba"), &a.pixels).unwrap();
        std::fs::write(dir.join("reference.rgba"), &b.pixels).unwrap();
        std::fs::write(dir.join("capture.txt"), format!(
            "{label} size={size:?} scale={scale} flipping={flipping}\n{mismatch:?}\nsemantic={semantic}\nCandidate {a:#?}\nReference {b:#?}"
        )).unwrap();
        panic!(
            "{label}, size={size:?}, scale={scale}, flipping={flipping}: {mismatch:?}, semantic={semantic}; artifacts: {}",
            dir.display()
        );
    }
}
