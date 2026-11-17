//! Cached raw hits are compared with the complete uncached App path pipeline.
use super::*;
#[path = "../../rsui-text/tests/support/mod.rs"]
mod font;

fn fixture(cx: &mut Cx, _: &()) -> Element {
    let state = cx.state(|| 0i64);
    let mode = cx.get(state);
    let mut target = Element::new(Role::Group).key("target").prop("dom_key", "target")
        .prop("position", "absolute").prop("left", if mode & 1 == 0 { 10 } else { 110 })
        .prop("top", 10).prop("width", 80).prop("height", 80)
        .prop("z_index", 1).prop("corner_radius", 40)
        .prop("inert", mode & 8 != 0);
    if mode & 4 != 0 { target = target.prop("rotate", 15); }
    let paragraph = Element::new(Role::Text).key("paragraph")
        .prop("position", "absolute").prop("left", 10).prop("top", 110)
        .prop("width", 190).prop("height", 40).prop("font_size", 16)
        .child(Element::new(Role::Text).key("first").name("AAAA "))
        .child(Element::new(Role::Text).key("second").name("BBBB"));
    let mut page = Element::new(Role::Group).prop("dom_key", "page")
        .prop("width", 240).prop("height", 180).prop("scroll_y", if mode & 16 == 0 { 0 } else { 40 })
        .action(ActionKind::Scroll).child(target).child(paragraph)
        .child(Element::new(Role::Group).prop("height", 500));
    if mode & 2 != 0 {
        page = page.child(Element::new(Role::Group).key("menu").prop("overlay", "inset")
            .prop("left", 20).prop("top", 20).prop("width", 60).prop("height", 60).prop("z_index", 9));
    }
    cx.on(Element::new(Role::Window).key("root").child(page), ActionKind::SetValue, move |cx, value| {
        if let Some(PropValue::Int(value)) = value { cx.set(state, *value); }
    })
}

fn new(enabled: bool) -> App {
    let mut app = App::new(fixture, (), TextEngine::new(font::plain_font()), (240,180), 1.0);
    app.hit_cells_enabled = enabled;
    app.update(); app.scene();
    app
}

fn compare(candidate: &mut App, reference: &mut App, x: f32, y: f32) -> Option<Vec<u32>> {
    let expected = reference.path_at(x,y);
    let actual = candidate.path_at_inner(x,y,true);
    assert_eq!(actual, expected, "point ({x},{y})");
    actual
}

fn change(app: &mut App, mode: i64) {
    app.runtime.dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(mode))).unwrap();
    app.update(); app.scene();
}

#[test]
fn cache_reuses_raw_hits_but_rechecks_round_corners_at_each_point() {
    let (mut candidate, mut reference) = (new(true), new(false));
    let center = compare(&mut candidate,&mut reference,50.0,50.0);
    let corner = compare(&mut candidate,&mut reference,20.0,20.0);
    assert_ne!(center,corner,"rounded corner must be refined after cached rectangle hit");
    assert!(candidate.hit_cell_counts[0]>0,"must exercise an actual certificate reuse");
    for y in 1..179 { for x in (1..239).step_by(7) {
        compare(&mut candidate,&mut reference,x as f32,y as f32);
    }}
}

#[test]
fn text_children_are_refined_at_the_current_glyph_position() {
    let (mut candidate, mut reference) = (new(true), new(false));
    let mut found = std::collections::BTreeSet::new();
    for x in 12..110 {
        if let Some(path)=compare(&mut candidate,&mut reference,x as f32,120.0) { found.insert(path); }
    }
    assert!(found.len()>1,"must visit different text parts");
    assert!(candidate.hit_cell_counts[0]>0);
}

#[test]
fn root_scroll_menu_inert_resize_and_scale_invalidate_old_certificates() {
    let (mut candidate, mut reference) = (new(true), new(false));
    for mode in [0,1,3,2,0,8,16,0] {
        change(&mut reference,mode); change(&mut candidate,mode);
        for &(x,y) in &[(50.0,50.0),(51.0,51.0),(125.0,50.0),(25.0,125.0)] {
            compare(&mut candidate,&mut reference,x,y);
        }
    }
    for &(size,scale) in &[((300,200),1.0),((480,360),2.0),((240,180),1.0)] {
        reference.resize(size,scale);reference.scene();candidate.resize(size,scale);candidate.scene();
        for &(x,y) in &[(50.0,50.0),(51.0,51.0),(125.0,50.0)] {compare(&mut candidate,&mut reference,x,y);}
    }
    assert!(candidate.raw_hit_cell.is_some());
    let _=candidate.runtime_mut();
    assert!(candidate.raw_hit_cell.is_none(),"opaque mutable Runtime exposure must clear immediately");
}

#[test]
fn presses_capture_selection_and_animation_bypass_certificate_reuse() {
    let mut app=new(true);
    app.path_at_inner(50.0,50.0,true);app.path_at_inner(51.0,51.0,true);
    assert!(app.hit_cell_counts[0]>0);
    app.buttons=1;app.pointer_capture=Some(Key::from("target"));
    let before=app.hit_cell_counts;
    app.path_at_inner(52.0,52.0,true);
    assert_eq!(app.hit_cell_counts,before);assert!(app.raw_hit_cell.is_none());
    app.buttons=0;app.pointer_capture=None;app.selecting_text=true;
    app.path_at_inner(50.0,50.0,true);assert!(app.raw_hit_cell.is_none());
    app.selecting_text=false;app.hover_since=Some(Instant::now());
    app.path_at_inner(50.0,50.0,true);assert!(app.raw_hit_cell.is_none());
    app.hover_since=None;
    app.path_at(50.0,50.0);assert!(app.raw_hit_cell.is_none(),"non-hover callers retain full traversal");
}

#[test]
fn unsupported_rotations_fall_back_to_complete_hit_testing() {
    let (mut candidate, mut reference) = (new(true), new(false));
    change(&mut reference,4);change(&mut candidate,4);
    for y in (12..95).step_by(4) { for x in (12..95).step_by(4) {
        compare(&mut candidate,&mut reference,x as f32,y as f32);
        assert!(candidate.raw_hit_cell.is_none());
    }}
}

fn scrolling_fixture(cx: &mut Cx, _: &()) -> Element {
    let content=Element::new(Role::Group).prop("height",700)
        .child(Element::new(Role::Group).key("hit").prop("height",80).prop("z_index",1))
        .child(Element::new(Role::Group).prop("height",600));
    Element::new(Role::Window).child(cx.scroll_area("scroll",rsui_runtime::ScrollAxis::Vertical,content))
}

#[test]
fn real_wheel_scroll_and_public_pointer_moves_keep_current_paths() {
    let create=|enabled|{
        let mut app=App::new(scrolling_fixture,(),TextEngine::new(font::plain_font()),(240,180),1.0);
        app.hit_cells_enabled=enabled;app.update();app.scene();app
    };
    let (mut candidate,mut reference)=(create(true),create(false));
    for y in [30.0,31.0,32.0] {
        reference.pointer_move(50.0,y);candidate.pointer_move(50.0,y);
        assert_eq!(candidate.pointer_paths[0],reference.pointer_paths[0]);
    }
    assert!(candidate.hit_cell_counts[0]>0);
    for pixels in [50.0,100.0,-30.0] {
        reference.scroll_pixels(50.0,40.0,pixels,false,true).unwrap();reference.update();reference.scene();
        candidate.scroll_pixels(50.0,40.0,pixels,false,true).unwrap();candidate.update();candidate.scene();
        for y in [30.0,31.0,32.0,100.0] {compare(&mut candidate,&mut reference,50.0,y);}
    }
}

fn pulsing_fixture(cx: &mut Cx, visible: &bool) -> Element {
    let pulse=Element::new(Role::Group).key("pulse").prop("height",20)
        .prop("animation","2 0 inf normal forwards linear\n0 opacity n 0\n1 opacity n 1");
    let content=Element::new(Role::Group).prop("height",1200)
        .child(Element::new(Role::Group).prop("height",if *visible {20}else{1000}).prop("z_index",1))
        .child(pulse);
    Element::new(Role::Window).child(cx.scroll_area("scroll",rsui_runtime::ScrollAxis::Vertical,content))
}

#[test]
fn offscreen_proven_visual_pulse_allows_hits_but_visible_motion_does_not() {
    for visible in [false,true] {
        let mut app=App::new(pulsing_fixture,visible,TextEngine::new(font::plain_font()),(240,180),1.0);
        app.hit_cells_enabled=true;app.runtime.set_own_clock(true);app.tick(Instant::now());
        app.set_animations(true);app.update();app.scene();
        assert!(app.runtime.is_animating());
        assert_eq!(app.app_animating(),visible);
        app.path_at_inner(10.0,10.0,true);app.path_at_inner(11.0,11.0,true);
        if visible {assert!(app.raw_hit_cell.is_none());}
        else {assert!(app.hit_cell_counts[0]>0);}
    }
}

#[test]
fn advancing_offscreen_pulse_and_public_moves_match_uncached_hits_between_frames() {
    let start=Instant::now();
    let create=|enabled|{
        let mut app=App::new(pulsing_fixture,false,TextEngine::new(font::plain_font()),(240,180),1.0);
        app.hit_cells_enabled=enabled;app.runtime.set_own_clock(true);app.tick(start);
        app.set_animations(true);app.update();app.scene();app
    };
    let (mut candidate,mut reference)=(create(true),create(false));
    let mut phases=std::collections::BTreeSet::new();
    let mut changed_frames=0;
    for frame in 1..=16 {
        let x=20.0+frame as f32;
        let before=candidate.tree_changes;
        // Pointer handling, empty updates, actual clock advance and frame
        // preparation are interleaved as in a window; the pulse keeps moving.
        for app in [&mut reference,&mut candidate] {
            app.pointer_move(x,30.0);app.update();
            app.tick(start+Duration::from_millis(frame*40));app.update();app.scene();
            assert!(app.runtime.is_animating());
            assert!(!app.app_animating(),"only the proven offscreen pulse is moving");
        }
        if candidate.tree_changes!=before {changed_frames+=1;}
        let pulse=&candidate.runtime.root().children[0].children[1];
        let opacity=match pulse.props.get("opacity") {
            Some(PropValue::Float(value))=>*value,
            other=>panic!("missing animated opacity: {other:?}"),
        };
        phases.insert(opacity.to_bits());
        assert_eq!(pulse.props.get("opacity"),reference.runtime.root().children[0].children[1].props.get("opacity"));
        for offset in [0.0,0.25,0.5] {
            reference.pointer_move(x+offset,31.0);candidate.pointer_move(x+offset,31.0);
            let full=reference.path_at(x+offset,31.0);
            assert_eq!(Some(candidate.pointer_paths[0].clone()),full);
            assert_eq!(candidate.pointer_paths[0],reference.pointer_paths[0]);
            let cell=candidate.raw_hit_cell.as_ref().expect("offscreen pulse permits a certificate");
            assert_eq!(cell.key.tree_changes,candidate.tree_changes,"cached path must describe current sampled tree");
        }
    }
    assert!(phases.len()>8,"the animation clock must actually advance");
    assert!(changed_frames>8,"exercise new native roots, not one frozen frame");
    assert!(candidate.hit_cell_counts[1]>=changed_frames,"new sampled trees must revalidate raw hits");
    assert!(candidate.hit_cell_counts[0]>=16,"also exercise reuse between frame publications");
}

fn wheel_app(enabled: bool) -> App {
    let mut app=App::new(scrolling_fixture,(),TextEngine::new(font::plain_font()),(240,180),1.0);
    app.wheel_hit_enabled=enabled;app.hit_cells_enabled=false;
    app.update();app.scene();app
}

fn wheel_finished(app: &App) {
    assert!(!app.wheel_hit_active);assert!(app.wheel_hit_memo.is_none());
}

#[test]
fn wheel_scope_reuses_three_vertical_queries_and_two_horizontal_queries_only_within_one_event() {
    let (mut candidate,mut reference)=(wheel_app(true),wheel_app(false));
    for kind in [0,1,2,0] {
        let a=candidate.wheel_hit_counts;let b=reference.wheel_hit_counts;
        for app in [&mut reference,&mut candidate] {
            match kind {
                0=>app.scroll(40.0,40.0,1).unwrap(),
                1=>app.scroll_pixels(40.0,40.0,10.0,false,true).unwrap(),
                _=>app.scroll_across(40.0,40.0,1).unwrap(),
            }
            wheel_finished(app);app.update();app.scene();
        }
        let calls=if kind==2 {2}else{3};
        assert_eq!(candidate.wheel_hit_counts,[a[0]+calls-1,a[1]+1]);
        assert_eq!(reference.wheel_hit_counts,[b[0],b[1]+calls]);
        assert_eq!(candidate.outline(),reference.outline());
    }
    let before=candidate.wheel_hit_counts;
    candidate.scroll_pixels(40.0,40.0,0.0,false,true).unwrap();wheel_finished(&candidate);
    assert_eq!(candidate.wheel_hit_counts,before,"zero delta still closes scope without querying");
}

fn rejected_scroll_fixture(cx: &mut Cx, _: &()) -> Element {
    let content=Element::new(Role::Group).prop("height",700);
    let area=Element::new(Role::Group).key("scroll").prop("height",180).prop("width",240)
        .prop("scroll_y",0).child(content);
    let area=cx.on(area,ActionKind::Scroll,|cx,_|cx.reject("wheel rejected"));
    Element::new(Role::Window).child(area)
}

#[test]
fn wheel_errors_discard_memo_before_the_next_public_call() {
    let mut app=App::new(rejected_scroll_fixture,(),TextEngine::new(font::plain_font()),(240,180),1.0);
    app.wheel_hit_enabled=true;app.update();app.scene();
    for _ in 0..2 {
        let before=app.wheel_hit_counts;
        assert!(matches!(app.scroll(40.0,40.0,1),Err(DispatchError::Rejected(_))));
        wheel_finished(&app);
        assert_eq!(app.wheel_hit_counts,[before[0]+2,before[1]+1]);
    }
}

fn wheel_callback_fixture(cx: &mut Cx, behavior: &u8) -> Element {
    let state=cx.state(||false);
    let mut content=Element::new(Role::Group).prop("height",700);
    if cx.get(state) {content=content.child(Element::new(Role::Group).key("menu").prop("overlay","fill").prop("z_index",9));}
    let area=cx.scroll_area("scroll",rsui_runtime::ScrollAxis::Vertical,content);
    let behavior=*behavior;
    cx.on(Element::new(Role::Window).key("root").child(area),ActionKind::Custom(Symbol::new("wheel")),move|cx,_|{
        match behavior {
            0=>cx.prevent_default(),
            1=>cx.set(state,true),
            _=>{
                crate::geometry::window_size((242.0,180.0));
                crate::geometry::window_size((241.0,180.0));
            }
        }
    })
}

#[test]
fn cancelled_and_mutating_wheel_callbacks_keep_defaults_and_current_geometry() {
    for behavior in [0u8,1,2] {
        let create=|enabled|{
            let mut app=App::new(wheel_callback_fixture,behavior,TextEngine::new(font::plain_font()),(240,180),1.0);
            app.wheel_hit_enabled=enabled;app.update();app.scene();app
        };
        let (mut candidate,mut reference)=(create(true),create(false));
        for _ in 0..2 {
            for app in [&mut reference,&mut candidate] {app.scroll(40.0,40.0,1).unwrap();wheel_finished(app);app.update();app.scene();}
            assert_eq!(candidate.outline(),reference.outline());
        }
        if behavior==0 {assert_eq!(candidate.wheel_hit_counts,[0,2],"cancelled wheel has no default hit queries");}
        if behavior==1 {assert!(candidate.outline().contains("#menu"));}
        if behavior==2 {assert!(candidate.wheel_hit_counts[1]>=3,"geometry change after wheel callback must force a new raw query");}
    }
}

#[test]
fn scoped_memo_rechecks_root_geometry_resize_exposure_and_current_glyphs() {
    let (mut candidate,mut reference)=(new(true),new(false));
    candidate.wheel_hit_enabled=true;
    candidate.with_wheel_hit_memo(|app|{
        let mut parts=std::collections::BTreeSet::new();
        for x in [15.0,15.0,90.0,90.0] {
            let expected=reference.path_at(x,120.0);
            let actual=app.path_at(x,120.0);assert_eq!(actual,expected);
            if let Some(path)=actual {parts.insert(path);}
        }
        assert!(parts.len()>1,"different glyph parts must stay point-dependent");
        assert!(app.wheel_hit_counts[0]>=2,"same-point glyph queries reuse raw path only");
        for mode in [1,3,8,16,0] {
            change(&mut reference,mode);change(app,mode);
            assert_eq!(app.path_at(50.0,50.0),reference.path_at(50.0,50.0));
        }
        app.resize((300,200),1.5);app.scene();reference.resize((300,200),1.5);reference.scene();
        assert_eq!(app.path_at(50.0,50.0),reference.path_at(50.0,50.0));
        assert!(app.wheel_hit_memo.is_some());let _=app.runtime_mut();assert!(app.wheel_hit_memo.is_none());
        Ok(())
    }).unwrap();
    wheel_finished(&candidate);
}

fn textarea_fixture(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window).child(Element::new(Role::TextInput).key("field")
        .prop("rows",3).prop("height",40).prop("width",200)
        .prop("value","one\ntwo\nthree\nfour\nfive\nsix\nseven"))
}

#[test]
fn textarea_consumption_closes_the_scope_before_another_event() {
    let mut app=App::new(textarea_fixture,(),TextEngine::new(font::plain_font()),(240,180),1.0);
    app.wheel_hit_enabled=true;app.update();app.scene();
    app.scroll_pixels(20.0,20.0,10.0,false,true).unwrap();wheel_finished(&app);
    assert_eq!(app.wheel_hit_counts,[1,1]);
    assert!(app.resting_area.is_some(),"the textarea consumed the scroll");
    let before=app.wheel_hit_counts;
    app.scroll_pixels(20.0,20.0,5.0,false,true).unwrap();wheel_finished(&app);
    assert_eq!(app.wheel_hit_counts,[before[0]+1,before[1]+1]);
}
