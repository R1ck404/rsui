//! Bounded single-owner state shared by the explicit pilot and automatic App backend.
//! The automatic feature selects from patch transactions and existing interaction paths.
use super::*;
use rsui_core::Patch;

pub(super) struct PilotState {
    path: [u32; 32],
    len: u8,
    pub(super) full: bool,
    pub(super) painting: bool,
}

impl PilotState {
    #[cfg(feature = "automatic-render")]
    pub(super) fn automatic() -> Self {
        Self {
            path: [0; 32],
            len: 0,
            full: true,
            painting: false,
        }
    }

    #[cfg(feature = "automatic-render")]
    pub(super) fn select(&mut self, path: &[u32]) {
        if path.len() > self.path.len() {
            self.len = 0;
            self.full = true;
        } else if path != self.path() {
            self.path[..path.len()].copy_from_slice(path);
            self.len = path.len() as u8;
            // General repaint repairs both the previous and the new owner.
            self.full = true;
        }
    }

    #[cfg(feature = "automatic-render")]
    pub(super) fn automatic_patches(&mut self, patches: &[Patch]) -> bool {
        let mut selected: Option<&[u32]> = None;
        // Classify the WHOLE ordered transaction without resolving any path.
        let local = patches.len() <= 32
            && patches.iter().all(|patch| match patch {
                Patch::SetProp { path, name, .. } | Patch::RemoveProp { path, name } => {
                    if !matches!(
                        name.as_str(),
                        "background" | "hover_background" | "pressed_background" | "text_color"
                    ) || path.len() > self.path.len()
                        || selected.is_some_and(|old| old != &**path)
                    {
                        return false;
                    }
                    selected = Some(path);
                    true
                }
                Patch::SetActions { .. } => true,
                _ => false,
            });
        if !local {
            self.len = 0;
            self.full = true;
        } else if let Some(path) = selected {
            self.select(path);
        }
        local
    }

    pub(super) fn path(&self) -> &[u32] {
        &self.path[..self.len as usize]
    }

    #[cfg(not(feature = "automatic-render"))]
    pub(super) fn accepts(&mut self, patches: &[Patch]) -> bool {
        // Reject the entire transaction before resolving any path. Structural patches
        // use intermediate-tree positions, never final-tree owner identities.
        let local = patches.len() <= 32
            && patches.iter().all(|patch| match patch {
                Patch::SetProp { path, name, .. } | Patch::RemoveProp { path, name } => {
                    &**path == self.path()
                        && matches!(
                            name.as_str(),
                            "background" | "hover_background" | "pressed_background" | "text_color"
                        )
                }
                Patch::SetActions { .. } => true,
                _ => false,
            });
        self.full |= !local;
        local
    }
}

/// Experimental owner of an App. No mutable Scene or App reference can escape it.
/// Feature gated; not selected by production windows. Commands stay in Painter;
/// submission replays bounded contributors, using temporary comparison history on full repaint.
pub struct MutationPilot {
    app: App,
    renderer: Renderer,
}

impl MutationPilot {
    /// Enroll one leaf path (at most five levels). Structural changes invalidate
    /// enrollment; the next full paint captures the new occupant of this path.
    pub fn new(mut app: App, path: &[u32], renderer: Renderer) -> Self {
        app.renderer_replaced();
        assert!(path.len() <= 5, "pilot path depth exceeds fixed budget");
        let mut indices = [0; 32];
        indices[..path.len()].copy_from_slice(path);
        app.pilot = Some(PilotState {
            path: indices,
            len: path.len() as u8,
            full: true,
            painting: false,
        });
        Self { app, renderer }
    }

    /// Unsupported operations use this escape hatch, invalidating before mutation.
    /// The closure cannot return borrowed state.
    pub fn edit<R>(&mut self, edit: impl FnOnce(&mut App) -> R) -> R {
        let mut state = self.app.pilot.take().expect("owned app");
        state.full = true;
        let result = edit(&mut self.app);
        self.app.pilot = Some(state);
        // An arbitrary closure may have exported/drained resources to another GPU.
        self.app.renderer_replaced();
        result
    }

    pub fn dispatch(
        &mut self,
        path: &[u32],
        action: ActionKind,
        value: Option<PropValue>,
    ) -> Result<(), DispatchError> {
        self.app.runtime.dispatch(path, action, value)
    }

    pub fn click(&mut self, x: f32, y: f32) -> Result<(), DispatchError> {
        self.app.click(x, y)
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.drag_invalidation();
        self.app.pointer_move(x, y);
        self.drag_invalidation();
    }
    pub fn pointer_down(&mut self, x: f32, y: f32) {
        self.drag_invalidation();
        self.app.pointer_down(x, y, 1);
        self.drag_invalidation();
    }
    pub fn pointer_up(&mut self, x: f32, y: f32) -> Result<(), DispatchError> {
        self.drag_invalidation();
        self.app.pointer_up(x, y)
    }
    fn drag_invalidation(&mut self) {
        if self.app.dragging.is_some() {
            self.app.pilot.as_mut().expect("owned app").full = true;
        }
    }

    pub fn update(&mut self) -> Frame {
        self.app.update()
    }

    /// Prepare authoritative commands without exporting or cloning them.
    pub fn prepare(&mut self) {
        self.app.pilot.as_mut().expect("owned app").painting = true;
        self.app.prepare_scene::<true>();
        self.app.pilot.as_mut().expect("owned app").painting = false;
    }

    /// Draw without Present, allowing submission and presentation to be measured separately.
    pub fn draw(&mut self) -> Result<(), RenderError> {
        #[cfg(feature = "automatic-render")]
        {
            self.app.render_resources_reset = false;
        }
        self.renderer.draw_owned_pilot(&mut self.app.painter)
    }

    #[cfg(feature = "render-diagnostics")]
    pub fn draw_timed_for_diagnostics(
        &mut self,
    ) -> Result<
        (
            std::time::Duration,
            Option<std::time::Duration>,
            &'static str,
        ),
        RenderError,
    > {
        #[cfg(feature = "automatic-render")]
        {
            self.app.render_resources_reset = false;
        }
        self.renderer
            .time_draw_for_diagnostics(|r| r.draw_owned_pilot(&mut self.app.painter))
    }

    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }
    pub fn present(&mut self) -> Result<(), RenderError> {
        self.renderer.present()
    }
    pub fn resize(&mut self, size: (u32, u32), scale: f32) -> Result<(), RenderError> {
        self.app.pilot.as_mut().expect("owned app").full = true;
        self.app.resize(size, scale);
        self.renderer.resize(size)
    }
    pub fn replace_renderer(&mut self, renderer: Renderer) {
        self.app.pilot.as_mut().expect("owned app").full = true;
        self.app.renderer_replaced();
        self.renderer = renderer;
    }

    #[cfg(feature = "render-diagnostics")]
    pub fn scene_work(&self) -> rsui_render::diagnostics::SceneWork {
        self.app.scene_work()
    }

    #[cfg(feature = "render-diagnostics")]
    pub fn storage(&self) -> rsui_render::diagnostics::PainterStorage {
        self.app.render_storage()
    }

    pub fn metadata_bytes(&self) -> usize {
        size_of::<Option<PilotState>>() + self.app.painter.owned_pilot_metadata_bytes()
    }

    /// Read-only scalar observations; no command export or scene traversal.
    pub fn work(&self) -> (bool, usize, usize) {
        self.app.painter.owned_pilot_work()
    }
}
