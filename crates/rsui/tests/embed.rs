//! Where a window kept inside an element goes, as a web view in a page does: its box, what of that shows past what scrolls
//! around it, and what's drawn on top of it.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use rsui::App;
use rsui::embed::{Embedded, PixelBox, Place};
use rsui::layout::{Placement, with_margin};
use rsui::core::Props;
use rsui::prelude::*;
use rsui::text::TextEngine;
use support::plain_font;

const SIZE: (u32, u32) = (300, 200);

struct Nothing;

impl Embedded for Nothing {
    fn place(&mut self, _: isize, _: Option<&Place>) {}
}

fn app_of(render: fn(&mut Cx, &()) -> Element, scale: f32) -> App {
    let mut app = App::new(render, (), TextEngine::new(plain_font()), SIZE, scale);
    app.update();
    app.scene();
    app
}

fn view(width: f32, height: f32) -> Element {
    Element::new(Role::Group)
        .props(Props::from(Style::new().width(width).height(height)))
        .prop("embed", 7i64)
}

fn boxed(left: i32, top: i32, right: i32, bottom: i32) -> PixelBox {
    PixelBox { left, top, right, bottom }
}

fn padded(cx: &mut Cx, _: &()) -> Element {
    let _ = cx;
    Element::new(Role::Window)
        .props(style!(Style::new().padding(10.0)))
        .child(view(100.0, 50.0))
}

#[test]
fn nothing_is_worked_out_while_nothing_is_kept_anywhere() {
    let mut app = app_of(padded, 1.0);
    assert!(app.embedded_places().is_empty());
}

#[test]
fn it_goes_where_its_element_is_in_physical_pixels() {
    rsui::embed::embed(7, Box::new(Nothing));
    let mut app = app_of(padded, 1.5);
    let places = app.embedded_places();
    assert_eq!(places.len(), 1, "{places:?}");
    let (id, place) = &places[0];
    assert_eq!(*id, 7);
    assert_eq!(place.rect, boxed(15, 15, 165, 90));
    assert_eq!(place.shown, place.rect, "nothing cuts it off");
    assert!(place.covered.is_empty());
    rsui::embed::unembed(7);
}

fn rounded(cx: &mut Cx, _: &()) -> Element {
    let _ = cx;
    Element::new(Role::Window).child(view(100.0, 50.0).prop("corner_radius", 8.0))
}

#[test]
fn its_corners_are_as_round_as_its_elements_in_physical_pixels() {
    rsui::embed::embed(7, Box::new(Nothing));
    let mut app = app_of(rounded, 1.5);
    let places = app.embedded_places();
    assert_eq!(places[0].1.radius, 12);
    let mut app = app_of(padded, 1.0);
    assert_eq!(app.embedded_places()[0].1.radius, 0, "square unless it says");
    rsui::embed::unembed(7);
}

fn scrolled(cx: &mut Cx, _: &()) -> Element {
    let content = Element::new(Role::Group)
        .props(style!(Style::new().flex(1.0)))
        .child(Element::new(Role::Group).props(style!(Style::new().width(200.0).height(120.0))))
        .child(view(200.0, 100.0))
        .child(Element::new(Role::Group).props(style!(Style::new().width(200.0).height(300.0))));
    let area = cx.scroll_area("area", ScrollAxis::Vertical, content);
    // What's below it leaves the area 150 pixels of the window.
    let below = Element::new(Role::Group).props(style!(Style::new().width(300.0).height(50.0)));
    Element::new(Role::Window).child(area).child(below)
}

#[test]
fn what_scrolls_around_it_moves_it_and_cuts_it_off() {
    rsui::embed::embed(7, Box::new(Nothing));
    let mut app = app_of(scrolled, 1.0);
    let places = app.embedded_places();
    let place = &places[0].1;
    assert_eq!(place.rect.top, 120);
    assert_eq!(place.shown.bottom, 150, "the area ends there: {place:?}");
    // Three rows of the wheel are 120 pixels.
    app.scroll(50.0, 50.0, 3).unwrap();
    app.update();
    app.scene();
    let place = app.embedded_places()[0].1.clone();
    assert!(place.rect.top < 120, "it moved up with what's around it: {place:?}");
    assert_eq!(place.rect.height(), 100);
    assert_eq!(place.shown, place.rect.intersect(&boxed(0, 0, 300, 150)));
    // Scrolled away entirely, it has no place, and is hidden.
    app.scroll(50.0, 50.0, 10).unwrap();
    app.update();
    app.scene();
    assert!(app.embedded_places().is_empty());
    rsui::embed::unembed(7);
}

fn covered(cx: &mut Cx, _: &()) -> Element {
    let _ = cx;
    let menu = Element::new(Role::Group)
        .props(style!(
            Style::new().width(80.0).height(40.0).overlay(Placement::Below),
            Paint::new().background(Color::hex(0x202020))
        ))
        .child(Element::new(Role::Text).name("A menu"));
    let see_through = Element::new(Role::Group).props(style!(Style::new().width(300.0).height(200.0).overlay(Placement::Fill)));
    let button = Element::new(Role::Button)
        .key("open")
        .name("Open")
        .props(style!(Style::new().width(60.0).height(20.0)))
        .child(menu);
    Element::new(Role::Window).child(button).child(view(200.0, 100.0)).child(see_through)
}

#[test]
fn what_is_drawn_on_top_of_it_is_cut_out_of_it_unless_it_can_be_seen_through() {
    rsui::embed::embed(7, Box::new(Nothing));
    let mut app = app_of(covered, 1.0);
    let places = app.embedded_places();
    let place = &places[0].1;
    assert_eq!(place.rect, boxed(0, 20, 200, 120));
    // The menu below the button is drawn after everything else, over the top of it. The overlay that covers the whole
    // window draws nothing, so it shows through that.
    assert_eq!(place.covered, vec![boxed(0, 20, 80, 60)], "{place:?}");
    rsui::embed::unembed(7);
}

fn faded(cx: &mut Cx, _: &()) -> Element {
    let _ = cx;
    let cover = Element::new(Role::Group).props(with_margin(
        style!(
            Style::new().width(100.0).height(40.0),
            Paint::new().background(Color::hex(0x202020)).opacity(0.0)
        ),
        -100.0,
        0.0,
        0.0,
        0.0,
    ));
    Element::new(Role::Window).child(view(200.0, 100.0)).child(cover)
}

#[test]
fn what_is_entirely_faded_out_covers_nothing() {
    rsui::embed::embed(7, Box::new(Nothing));
    let mut app = app_of(faded, 1.0);
    let place = app.embedded_places()[0].1.clone();
    assert!(place.covered.is_empty(), "{place:?}");
    rsui::embed::unembed(7);
}

fn dimmed(cx: &mut Cx, _: &()) -> Element {
    let _ = cx;
    // A backdrop that dims everything, with a panel on it, as a dialog over a page has.
    let panel = Element::new(Role::Group).props(style!(
        Style::new().width(100.0).height(40.0),
        Paint::new().background(Color::hex(0xffffff))
    ));
    let backdrop = Element::new(Role::Group)
        .props(style!(
            Style::new().width(300.0).height(200.0).overlay(Placement::Fill),
            Paint::new().background(Color::rgba(0, 0, 0, 60))
        ))
        .child(panel);
    Element::new(Role::Window).child(view(200.0, 100.0).prop("embed_still", true)).child(backdrop)
}

#[test]
fn what_is_seen_through_over_it_is_said_apart_from_what_covers_it_and_so_is_a_still_picture_of_it() {
    rsui::embed::embed(7, Box::new(Nothing));
    let mut app = app_of(dimmed, 1.0);
    let place = app.embedded_places()[0].1.clone();
    assert_eq!(place.see_through, vec![place.shown], "the backdrop is over all of it: {place:?}");
    assert_eq!(place.covered, vec![boxed(0, 0, 100, 40)], "the panel covers it: {place:?}");
    assert!(place.still, "its element draws a still picture of it");
    rsui::embed::unembed(7);
}
