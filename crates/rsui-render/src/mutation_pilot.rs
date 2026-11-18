//! Fixed-size enrollment record; ranges refer to the sole authoritative Scene.
use super::*;

pub(crate) struct PilotPaint {
    // Only command vectors live here. Resource queues stay in the current Scene.
    pub(crate) history: Scene,
    pub(crate) pending: Option<((u64, u64), PixelRect)>,
    plan_ready: bool,
    pub(crate) epoch: u64,
    pub(crate) revision: u64,
    pub(crate) region: PixelRect,
    pub(crate) contributors: Vec<(u32, u32)>,
    pub(super) target: usize, // Used only during enrollment, never as persistent identity.
    first: u32,
    text: u32,
    end: u32,
    inherited: Color,
    pub(super) captures: u32,
    pub(crate) valid: bool,
    pub(super) fast: bool,
}

fn eligible(e: &Element, look: Look) -> bool {
    e.children.is_empty()
        && matches!(e.role, Role::Button | Role::Text)
        && e.name.is_ascii()
        && !look.shadowed
        && !look.arranged
        && !look.transformed
        && !look.filtered
        && !look.shaped
        && !look.gradient
        && !look.background_image
        && !look.text_shadow
        && look.faded == 0.0
        && look.background.is_some_and(|c| c.a > 0)
        && look.hover.is_none_or(|c| c.a > 0)
        && look.pressed.is_none_or(|c| c.a > 0)
}

pub(super) fn interaction_safe(e: &Element, i: Interaction<'_>) -> bool {
    i.hovered.is_none_or(|v| std::ptr::eq(v, e))
        && i.pressed.is_none_or(|v| std::ptr::eq(v, e))
        && i.hover_amount == 1.0
        && i.leaving.is_none()
        && i.ring.is_none()
        && i.editing.is_none()
        && i.text_selection.is_none()
        && i.tooltip.is_none()
        && i.overlay.is_none()
        && i.resting_area.is_none()
}

impl Default for PilotPaint {
    fn default() -> Self {
        Self {
            history: Scene::default(),
            pending: None,
            plan_ready: false,
            epoch: 0,
            revision: 0,
            region: crate::gpu::NO_AREA,
            contributors: Vec::new(),
            target: 0,
            first: 0,
            text: 0,
            end: 0,
            inherited: Color::WHITE,
            captures: 0,
            valid: false,
            fast: false,
        }
    }
}
impl PilotPaint {
    pub(crate) fn invalidate(&mut self) {
        self.valid = false;
        self.plan_ready = false;
        if let Some((stamp, _)) = &mut self.pending {
            *stamp = (0, 0);
        }
        self.epoch = 0;
        self.contributors.clear();
    }

    pub(super) fn capture(
        &mut self,
        e: &Element,
        look: Look,
        range: [usize; 3],
        inherited: Color,
        safe: bool,
    ) {
        if self.target != std::ptr::from_ref(e) as usize {
            return;
        }
        self.captures += 1;
        self.valid = self.captures == 1
            && safe
            && eligible(e, look)
            && range[2] <= u32::MAX as usize
            && range[0] < range[1];
        [self.first, self.text, self.end] = range.map(|n| n as u32);
        self.inherited = inherited;
    }
}

impl Painter {
    #[doc(hidden)]
    pub fn has_owned_palette(&self) -> bool {
        self.pilot.valid
    }

    #[doc(hidden)]
    pub fn enroll_owned_pilot(&mut self, target: Option<&Element>) {
        self.pilot.target = target.map_or(0, |e| std::ptr::from_ref(e) as usize);
        self.pilot.valid = false;
        self.pilot.plan_ready = false;
        self.pilot.contributors.clear();
    }

    #[doc(hidden)]
    pub fn update_owned_pilot(&mut self, e: &Element, interaction: Interaction<'_>) -> bool {
        self.pilot.fast = false;
        let look = Look::read(&e.props, &NAMES);
        if !self.pilot.valid || !eligible(e, look) || !interaction_safe(e, interaction) {
            return false;
        }
        if !self.pilot.plan_ready {
            self.build_owned_plan();
            if !self.pilot.valid {
                return false;
            }
        }
        let background = look
            .pressed
            .filter(|_| interaction.pressed.is_some())
            .or(look.hover.filter(|_| interaction.hovered.is_some()))
            .or(look.background)
            .expect("eligible background");
        let mut changed =
            self.scene.quads[self.pilot.first as usize].color != background.premultiplied();
        self.scene.quads[self.pilot.first as usize].color = background.premultiplied();
        let color = look
            .text_color
            .unwrap_or(self.pilot.inherited)
            .premultiplied();
        for quad in &mut self.scene.quads[self.pilot.text as usize..self.pilot.end as usize] {
            changed |= quad.color != color;
            quad.color = color;
        }
        #[cfg(feature = "render-diagnostics")]
        {
            self.work = crate::diagnostics::SceneWork {
                serial: self.work.serial.saturating_add(1),
                path: crate::diagnostics::ScenePath::OwnedPalette,
                output: crate::diagnostics::Commands::of(&self.scene),
                atlas_before: self.atlas_observation(),
                atlas_after: self.atlas_observation(),
                ..Default::default()
            };
        }
        if changed {
            self.pilot.revision = self
                .pilot
                .revision
                .checked_add(1)
                .expect("revision exhausted");
        }
        self.pilot.fast = true;
        true
    }

    #[doc(hidden)]
    pub fn owned_pilot_metadata_bytes(&self) -> usize {
        size_of::<PilotPaint>() + self.pilot.contributors.capacity() * size_of::<(u32, u32)>()
    }

    #[cfg(feature = "render-diagnostics")]
    #[doc(hidden)]
    pub fn owned_pilot_metadata_storage(&self) -> (usize, usize) {
        (
            size_of::<PilotPaint>(),
            self.pilot.contributors.capacity() * size_of::<(u32, u32)>(),
        )
    }

    #[doc(hidden)]
    pub fn owned_pilot_work(&self) -> (bool, usize, usize) {
        (
            self.pilot.fast,
            if self.pilot.fast {
                (self.pilot.end - self.pilot.text) as usize + 1
            } else {
                self.scene.quads.len()
            },
            self.scene.quads.capacity() * size_of::<Quad>(),
        )
    }
}

impl Painter {
    /// Paint without exporting mutable commands. Only this entry can establish a plan.
    #[doc(hidden)]
    pub fn paint_owned_pilot(
        &mut self,
        root: &Node,
        layout: &Layout,
        text: &mut TextEngine,
        scale: f32,
        size: (u32, u32),
        interaction: Interaction<'_>,
    ) {
        self.begin_owned_repaint();
        self.paint_traversal::<true>(root, layout, text, scale, size, interaction);
        self.finish_owned_repaint();
    }

    /// Reuse the caret cache while preserving comparison history and ownership.
    #[doc(hidden)]
    pub fn show_owned_caret(&mut self, show: bool) -> bool {
        if !self.can_show_caret() {
            return false;
        }
        self.begin_owned_repaint();
        self.show_caret_internal(show).expect("kept caret");
        self.pilot.fast = false;
        self.pilot.valid = false;
        self.finish_owned_repaint();
        true
    }

    fn begin_owned_repaint(&mut self) {
        // Actual full-paint boundary: save once, even across multiple prepares.
        // Enrollment does not reset the previous stamp/region before this move.
        if self.pilot.pending.is_none() {
            self.pilot.pending = Some(((self.pilot.epoch, self.pilot.revision), self.pilot.region));
            std::mem::swap(&mut self.scene.quads, &mut self.pilot.history.quads);
            std::mem::swap(&mut self.scene.batches, &mut self.pilot.history.batches);
            std::mem::swap(
                &mut self.scene.transforms,
                &mut self.pilot.history.transforms,
            );
            std::mem::swap(&mut self.scene.filters, &mut self.pilot.history.filters);
            std::mem::swap(&mut self.scene.round_clips, &mut self.pilot.history.round_clips);
        }
    }

    fn finish_owned_repaint(&mut self) {
        // Unique across Painters and renderer replacements; no pointer identity proof.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        self.pilot.epoch = NEXT
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |v| v.checked_add(1),
            )
            .expect("epoch exhausted");
        self.pilot.revision = 0;
        self.pilot.contributors.clear();
        self.pilot.plan_ready = false;
        self.pilot.valid &= self.scene.transforms.is_empty()
            && self.scene.filters.is_empty()
            && self.scene.round_clips.is_empty();
    }

    fn build_owned_plan(&mut self) {
        use crate::gpu::{NO_AREA, area_of, intersect, is_empty, union};
        self.pilot.plan_ready = true;
        let mut region = NO_AREA;
        for batch in &self.scene.batches {
            let start = batch.first.max(self.pilot.first);
            let end = (batch.first + batch.count).min(self.pilot.end);
            for i in start..end {
                region = union(
                    region,
                    intersect(area_of(&self.scene.quads[i as usize]), batch.clip),
                );
            }
        }
        self.pilot.region = region;
        // 512 * (8 index + 32 quad + 32 batch) <= 36 KiB, below 64 KiB.
        // Store only indices; selected commands are materialized in the renderer.
        for (b, batch) in self.scene.batches.iter().enumerate() {
            for i in batch.first..batch.first + batch.count {
                if !is_empty(intersect(
                    area_of(&self.scene.quads[i as usize]),
                    intersect(batch.clip, region),
                )) {
                    if self.pilot.contributors.len() == 512 {
                        self.pilot.valid = false;
                        self.pilot.contributors.clear();
                        return;
                    }
                    self.pilot.contributors.push((i, b as u32));
                }
            }
        }
    }
}

#[cfg(all(test, feature = "render-diagnostics"))]
mod tests {
    use super::*;
    use crate::Renderer;
    #[test]
    fn repaint_moves_commands_once_and_leaves_resource_queues_in_place() {
        let root = Element::new(Role::Window)
            .child(
                Element::new(Role::Button)
                    .name("ABCD")
                    .prop("width", 70)
                    .prop("height", 25)
                    .prop("background", 0xff0000ff_i64),
            )
            .into_node();
        let mut painter = Painter::new();
        let mut text = TextEngine::new(super::super::diagnostic_font::plain_font());
        let mut layout = Layout::new();
        layout.compute(&root, rsui_layout::Size::new(100.0, 80.0), &mut text);
        let mut gpu = Renderer::offscreen((100, 80)).unwrap();
        painter.enroll_owned_pilot(Some(&root.children[0]));
        painter.paint_owned_pilot(
            &root,
            &layout,
            &mut text,
            1.0,
            (100, 80),
            Interaction::default(),
        );
        gpu.draw_owned_pilot(&mut painter).unwrap();
        gpu.present().unwrap();
        let authoritative = painter.scene.quads.as_ptr();
        painter.scene.dropped_images.push(777);
        painter.scene.image_uploads.push(ImageUpload {
            id: 778,
            width: 1,
            height: 1,
            pixels: vec![255; 4],
            changes: None,
        });
        for _ in 0..2 {
            painter.enroll_owned_pilot(Some(&root.children[0]));
            painter.paint_owned_pilot(
                &root,
                &layout,
                &mut text,
                1.0,
                (100, 80),
                Interaction::default(),
            );
            assert_eq!(painter.pilot.history.quads.as_ptr(), authoritative);
            assert!(!painter.pilot.history.quads.is_empty());
            assert!(painter.pilot.history.uploads.is_empty());
            assert!(painter.pilot.history.image_uploads.is_empty());
            assert!(painter.pilot.history.dropped_images.is_empty());
            assert!(painter.scene.dropped_images.contains(&777));
            assert!(painter.scene.image_uploads.iter().any(|u| u.id == 778));
        }
    }

    #[test]
    fn contributor_overflow_uses_direct_fallback_with_bounded_storage() {
        let mut root = Element::new(Role::Window).child(
            Element::new(Role::Button)
                .name("Save")
                .prop("width", 80)
                .prop("height", 40)
                .prop("background", 0xff000080_i64),
        );
        for _ in 0..600 {
            root = root.child(
                Element::new(Role::Text)
                    .prop("position", "absolute")
                    .prop("width", 60)
                    .prop("height", 30)
                    .prop("background", 0x00ff0080_i64),
            );
        }
        let root = root.into_node();
        let mut painter = Painter::new();
        let mut text = TextEngine::new(super::super::diagnostic_font::plain_font());
        let mut layout = Layout::new();
        layout.compute(&root, rsui_layout::Size::new(100.0, 80.0), &mut text);
        painter.enroll_owned_pilot(Some(&root.children[0]));
        painter.paint_owned_pilot(
            &root,
            &layout,
            &mut text,
            1.0,
            (100, 80),
            Interaction {
                hover_amount: 1.0,
                ..Default::default()
            },
        );
        assert!(!painter.update_owned_pilot(
            &root.children[0],
            Interaction {
                hover_amount: 1.0,
                ..Default::default()
            }
        ));
        assert!(!painter.pilot.valid);
        assert!(painter.pilot.contributors.is_empty());
        assert!(painter.pilot.contributors.capacity() <= 512);
        assert!(
            512 * (size_of::<Quad>() + size_of::<Batch>() + size_of::<(u32, u32)>()) < 64 * 1024
        );
        let mut gpu = Renderer::offscreen((100, 80)).unwrap();
        gpu.draw_owned_pilot(&mut painter).unwrap();
        assert_eq!(
            gpu.draw_work().outcome,
            crate::diagnostics::DrawOutcome::Full
        );
    }
    #[test]
    fn bounded_plan_transparency_pending_and_public_escapes() {
        for flipping in [false, true] {
            let make = || {
                if flipping {
                    Renderer::offscreen_flipping((100, 80))
                } else {
                    Renderer::offscreen((100, 80))
                }
                .unwrap()
            };
            let mut gpu = make();
            let mut reference = make();
            reference.set_always_redraw(true);
            let leaf = Element::new(Role::Button)
                .name("Save")
                .prop("width", 60)
                .prop("height", 30)
                .prop("background", 0x2080e080_i64)
                .prop("border_width", 2)
                .prop("border_color", 0xff000080_i64);
            let root = Element::new(Role::Window)
                .prop("background", 0xffffffff_i64)
                .child(
                    Element::new(Role::Text)
                        .prop("position", "absolute")
                        .prop("width", 80)
                        .prop("height", 40)
                        .prop("background", 0xffa00080_i64),
                )
                .child(leaf.clone())
                .child(
                    Element::new(Role::Text)
                        .name("over")
                        .prop("position", "absolute")
                        .prop("top", 8)
                        .prop("width", 40)
                        .prop("height", 20)
                        .prop("background", 0x00ff0080_i64),
                )
                .into_node();
            let mut painter = Painter::new();
            let mut text = TextEngine::new(super::super::diagnostic_font::plain_font());
            let mut layout = Layout::new();
            layout.compute(&root, rsui_layout::Size::new(100.0, 80.0), &mut text);
            painter.enroll_owned_pilot(Some(&root.children[1]));
            painter.paint_owned_pilot(
                &root,
                &layout,
                &mut text,
                1.0,
                (100, 80),
                Interaction {
                    hover_amount: 1.0,
                    ..Default::default()
                },
            );
            assert!(painter.pilot.valid);
            for step in 0..8 {
                if step > 0 {
                    let update = leaf.clone().prop(
                        "background",
                        if step % 2 == 0 {
                            0xff000040_i64
                        } else {
                            0x0000ff80_i64
                        },
                    );
                    assert!(painter.update_owned_pilot(
                        &update,
                        Interaction {
                            hover_amount: 1.0,
                            ..Default::default()
                        }
                    ));
                    // Multiple prepares must not acknowledge the pending mutation.
                    assert!(painter.update_owned_pilot(
                        &update,
                        Interaction {
                            hover_amount: 1.0,
                            ..Default::default()
                        }
                    ));
                }
                let mut expected = painter.scene.clone();
                gpu.draw_owned_pilot(&mut painter).unwrap();
                if step == 3 {
                    gpu.draw_owned_pilot(&mut painter).unwrap();
                }
                gpu.present().unwrap();
                reference.render(&mut expected, Color::WHITE).unwrap();
                assert_eq!(
                    gpu.read_pixels().unwrap(),
                    reference.read_pixels().unwrap(),
                    "step {step}"
                );
                gpu.draw_owned_pilot(&mut painter).unwrap();
                assert!(gpu.showed_nothing());
                gpu.present().unwrap();
                gpu.present().unwrap();
                assert_eq!(gpu.read_pixels().unwrap(), reference.read_pixels().unwrap());
            }
            assert!(painter.owned_pilot_metadata_bytes() < 8192);
            painter.last_scene().quads.clear();
            assert!(!painter.update_owned_pilot(
                &leaf,
                Interaction {
                    hover_amount: 1.0,
                    ..Default::default()
                }
            ));
            // A public full paint's returned Scene can be mutated, so cannot enroll.
            painter.enroll_owned_pilot(Some(&root.children[1]));
            painter
                .paint_with(
                    &root,
                    &layout,
                    &mut text,
                    1.0,
                    (100, 80),
                    Interaction {
                        hover_amount: 1.0,
                        ..Default::default()
                    },
                )
                .batches
                .clear();
            assert!(!painter.update_owned_pilot(
                &leaf,
                Interaction {
                    hover_amount: 1.0,
                    ..Default::default()
                }
            ));
            gpu.draw_owned_pilot(&mut painter).unwrap();
            gpu.present().unwrap();
            painter.enroll_owned_pilot(Some(&root.children[1]));
            painter.paint_owned_pilot(
                &root,
                &layout,
                &mut text,
                1.0,
                (100, 80),
                Interaction {
                    hover_amount: 1.0,
                    ..Default::default()
                },
            );
            painter.show_caret(false);
            assert!(!painter.update_owned_pilot(
                &leaf,
                Interaction {
                    hover_amount: 1.0,
                    ..Default::default()
                }
            ));
        }
    }
}
