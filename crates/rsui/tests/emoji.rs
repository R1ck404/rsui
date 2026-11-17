//! Emoji show in color, and everything shows up with no fonts chosen, in a real app.

use rsui::prelude::*;
use rsui::render::Scene;
use rsui::{App, default_text_engine};

const SIZE: (u32, u32) = (400, 200);

fn showing(_: &mut Cx, words: &&'static str) -> Element {
    Element::new(Role::Window).child(Element::new(Role::Text).name(*words))
}

/// What a frame showing this has in it.
fn scene_of(words: &'static str) -> Option<Scene> {
    let engine = default_text_engine().ok()?;
    let mut app = App::new(showing, words, engine, SIZE, 1.0);
    app.update();
    Some(app.scene().clone())
}

fn has_emoji_font() -> bool {
    std::path::Path::new("C:/Windows/Fonts/seguiemj.ttf").exists()
}

/// The quads that are glyphs in the atlas.
fn glyph_count(scene: &Scene) -> usize {
    scene
        .batches
        .iter()
        .filter(|batch| batch.image == 0)
        .flat_map(|batch| &scene.quads[batch.first as usize..(batch.first + batch.count) as usize])
        .filter(|quad| quad.glyph[2] > 0)
        .count()
}

fn picture_count(scene: &Scene) -> usize {
    scene
        .batches
        .iter()
        .filter(|batch| batch.image != 0)
        .count()
}

#[test]
fn an_emoji_is_a_picture_with_many_colors_in_it() {
    if !has_emoji_font() {
        return;
    }
    let scene = scene_of("\u{1F600}").unwrap();
    assert_eq!(picture_count(&scene), 1);
    assert_eq!(scene.image_uploads.len(), 1);
    let mut colors: Vec<&[u8]> = scene.image_uploads[0].pixels.chunks_exact(4).collect();
    colors.sort_unstable();
    colors.dedup();
    assert!(colors.len() > 20, "only {} colors", colors.len());
}

#[test]
fn the_same_emoji_twice_is_one_picture_drawn_twice() {
    if !has_emoji_font() {
        return;
    }
    let scene = scene_of("\u{1F600}\u{1F600}\u{1F600}").unwrap();
    assert_eq!(picture_count(&scene), 3);
    assert_eq!(scene.image_uploads.len(), 1);
}

#[test]
fn plain_text_stays_plain_with_an_emoji_font_about() {
    let Some(scene) = scene_of("Hello") else {
        return;
    };
    assert_eq!(picture_count(&scene), 0);
    assert_eq!(glyph_count(&scene), 5);
}

#[test]
fn what_comes_before_and_after_an_emoji_stays_in_order() {
    if !has_emoji_font() {
        return;
    }
    let scene = scene_of("A\u{1F600}B").unwrap();
    let images: Vec<bool> = scene.batches.iter().map(|batch| batch.image != 0).collect();
    let at = images.iter().position(|image| *image).unwrap();
    assert!(at > 0 && at + 1 < images.len(), "{images:?}");
    assert_eq!(glyph_count(&scene), 2);
}

#[test]
fn a_keycap_comes_out_as_one_picture() {
    if !has_emoji_font() {
        return;
    }
    let scene = scene_of("1\u{FE0F}\u{20E3}").unwrap();
    assert_eq!(picture_count(&scene), 1);
    assert_eq!(glyph_count(&scene), 0);
}

fn fading(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window).child(
        Element::new(Role::Text)
            .name("\u{1F600}")
            .props(style!(Paint::new().text(Color::rgba(0, 0, 0, 128)))),
    )
}

#[test]
fn see_through_text_has_see_through_emoji() {
    if !has_emoji_font() {
        return;
    }
    let mut app = App::new(fading, (), default_text_engine().unwrap(), SIZE, 1.0);
    app.update();
    let scene = app.scene().clone();
    let batch = scene.batches.iter().find(|batch| batch.image != 0).unwrap();
    assert_eq!(scene.quads[batch.first as usize].color[3], 128);
}

#[test]
fn chinese_and_japanese_show_up_with_no_fonts_chosen() {
    if !std::path::Path::new("C:/Windows/Fonts/msyh.ttc").exists() {
        return;
    }
    assert_eq!(glyph_count(&scene_of("\u{6F22}\u{5B57}").unwrap()), 2);
    assert_eq!(glyph_count(&scene_of("\u{3042}\u{30A2}").unwrap()), 2);
    assert_eq!(glyph_count(&scene_of("\u{D55C}\u{AE00}").unwrap()), 2);
}
