//! Differential decisions across every sampled point inside each certificate.
use super::*;
use rsui_core::Role;

const VIEW: Rect = Rect::new(0.0, 0.0, 400.0, 300.0);

fn seeded(depth: usize, seed: usize) -> (LayoutBox, Node) {
    let flags = [
        IS_ABSOLUTE,
        SCROLLS,
        IS_OVERLAY,
        ABSOLUTE_IS_INERT,
        ABSOLUTE_TAKES_POINTER,
        CONTENTS,
        STACKED,
        OVERLAY_IS_INERT,
    ];
    let mut current = LayoutBox {
        x: if depth == 3 {
            0.0
        } else {
            ((seed * 17) % 120) as f32 - 35.25
        },
        y: if depth == 3 {
            0.0
        } else {
            ((seed * 31) % 90) as f32 - 20.125
        },
        width: if depth == 3 {
            400.0
        } else {
            40.0 + (seed % 3) as f32 * 30.0
        },
        height: if depth == 3 {
            300.0
        } else {
            35.0 + (seed % 5) as f32 * 10.0
        },
        flags: flags
            .iter()
            .enumerate()
            .filter(|(i, _)| seed & (1 << i) != 0)
            .fold(0, |bits, (_, flag)| bits | flag),
        ..LayoutBox::default()
    };
    let mut node = Element::new(Role::Group)
        .prop("z_index", (seed % 7) as i64)
        .prop("test_translation", (seed % 5) as i64);
    if depth > 0 {
        for offset in [0, 37, 71] {
            let (child, child_node) = seeded(depth - 1, (seed + offset) % 256);
            current.children.push(child);
            node.children.push(child_node);
        }
    }
    current.update_contains_absolute();
    current.update_contains_overlay();
    (current, node.into_node())
}

fn translation(node: &Element, _: Rect) -> Option<[f32; 6]> {
    match node.props.get("test_translation") {
        Some(rsui_core::PropValue::Int(value)) if *value != 0 => Some([
            1.0,
            0.0,
            0.0,
            1.0,
            *value as f32 * 0.125,
            -*value as f32 * 0.1,
        ]),
        _ => None,
    }
}

#[test]
fn certificates_preserve_reference_hits_across_all_inside_samples() {
    let mut certificates = 0;
    let mut samples = 0;
    for seed in 0..256 {
        let (root, node) = seeded(3, seed);
        let layout = Layout {
            root,
            ..Layout::default()
        };
        for y in [0.0, 0.125, 5.0, 31.25, 79.5, 150.0, 299.875, 300.0] {
            for x in [0.0, 0.125, 5.0, 41.25, 99.5, 200.0, 399.875, 400.0] {
                let (hit, certificate) =
                    layout.hit_test_turned_certified(&node, x, y, &translation, VIEW);
                assert_eq!(hit, layout.hit_test_turned(&node, x, y, &translation));
                let Some(region) = certificate else {
                    continue;
                };
                certificates += 1;
                assert!(region.contains(x, y));
                let [left, top, right, bottom] = region.bounds();
                assert!(left >= 0.0 && right <= 400.0 && top >= 0.0 && bottom <= 300.0);
                for fx in [0.000_001, 0.01, 0.25, 0.5, 0.9, 0.99, 0.999_999] {
                    for fy in [0.000_001, 0.01, 0.25, 0.5, 0.9, 0.99, 0.999_999] {
                        let px = (left + (right - left) * fx) as f32;
                        let py = (top + (bottom - top) * fy) as f32;
                        if region.contains(px, py) {
                            assert_eq!(
                                layout.hit_test_turned(&node, px, py, &translation),
                                hit,
                                "seed {seed}: source {x},{y}, sampled {px},{py}, region {region:?}"
                            );
                            samples += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(certificates > 1_000, "{certificates}");
    assert!(samples > 40_000, "{samples}");
}

#[test]
fn unsupported_transforms_inputs_and_boundaries_decline_without_changing_hits() {
    let (root, node) = seeded(3, 17);
    let layout = Layout {
        root,
        ..Layout::default()
    };
    for matrix in [
        [2.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        [1.0, 0.5, 0.0, 1.0, 0.0, 0.0],
        [0.0, -1.0, 1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 1.0, f32::INFINITY, 0.0],
        [1.0, 0.0, 0.0, 1.0, f32::NAN, 0.0],
    ] {
        let undo = |_: &Element, _: Rect| Some(matrix);
        let (hit, region) = layout.hit_test_turned_certified(&node, 50.0, 50.0, &undo, VIEW);
        assert_eq!(hit, layout.hit_test_turned(&node, 50.0, 50.0, &undo));
        assert!(region.is_none());
    }
    for (x, y, bounds) in [
        (0.0, 5.0, VIEW),
        (400.0, 5.0, VIEW),
        (5.0, 0.0, VIEW),
        (5.0, 300.0, VIEW),
        (f32::NAN, 5.0, VIEW),
        (5.0, f32::INFINITY, VIEW),
        (5.0, 5.0, Rect::new(0.0, 0.0, f32::INFINITY, 300.0)),
        (5.0, 5.0, Rect::new(0.0, 0.0, -10.0, 300.0)),
        (5.0, 5.0, Rect::new(0.0, 0.0, f32::MAX / 2.0, 300.0)),
    ] {
        let (hit, region) = layout.hit_test_turned_certified(&node, x, y, &translation, bounds);
        assert_eq!(hit, layout.hit_test_turned(&node, x, y, &translation));
        assert!(region.is_none(), "{x},{y}: {bounds:?}");
    }
}

#[test]
fn excessive_translation_depth_falls_back() {
    let mut root = LayoutBox {
        width: 100.0,
        height: 100.0,
        ..LayoutBox::default()
    };
    let mut node = Element::new(Role::Group);
    for _ in 0..70 {
        root = LayoutBox {
            width: 100.0,
            height: 100.0,
            children: vec![root].into(),
            ..LayoutBox::default()
        };
        node = Element::new(Role::Group).child(node);
    }
    let node = node.into_node();
    let layout = Layout {
        root,
        ..Layout::default()
    };
    let undo = |_: &Element, _: Rect| None;
    let (hit, region) = layout.hit_test_turned_certified(&node, 50.0, 50.0, &undo, VIEW);
    assert_eq!(hit, layout.hit_test_turned(&node, 50.0, 50.0, &undo));
    assert!(region.is_none());
}

#[test]
fn fractional_cancellation_large_offsets_and_depth64_keep_certified_edges_safe() {
    for (offset, size) in [
        (0.1f32, 1000.0),
        (0.125, 1000.0),
        (10_000_000.0, 1_000_000.0),
        (-0.0, 1000.0),
    ] {
        for depth in [0, 1, 16, 64] {
            let mut current = LayoutBox {
                x: offset,
                y: offset,
                width: size,
                height: size,
                ..LayoutBox::default()
            };
            let mut node = Element::new(Role::Group);
            for _ in 0..depth {
                current = LayoutBox {
                    x: offset,
                    y: offset,
                    width: size,
                    height: size,
                    children: vec![current].into(),
                    ..LayoutBox::default()
                };
                node = Element::new(Role::Group).child(node);
            }
            let node = node.into_node();
            let layout = Layout {
                root: current,
                ..Layout::default()
            };
            let undo = |_: &Element, _: Rect| Some([1.0, -0.0, 0.0, 1.0, offset, offset]);
            let viewport = Rect::new(0.0, 0.0, size, size);
            let (hit, region) =
                layout.hit_test_turned_certified(&node, size / 2.0, size / 2.0, &undo, viewport);
            let region = region.expect("wide interior should certify even at depth64");
            let [left, top, right, bottom] = region.bounds();
            for px in [
                (left as f32).next_up(),
                ((left + right) / 2.0) as f32,
                (right as f32).next_down(),
            ] {
                for py in [
                    (top as f32).next_up(),
                    ((top + bottom) / 2.0) as f32,
                    (bottom as f32).next_down(),
                ] {
                    if region.contains(px, py) {
                        assert_eq!(
                            layout.hit_test_turned(&node, px, py, &undo),
                            hit,
                            "offset {offset}, depth {depth}, sampled {px},{py}"
                        );
                    }
                }
            }
            assert!(!region.contains((left as f32).next_down(), size / 2.0));
            assert!(!region.contains((right as f32).next_up(), size / 2.0));
        }
    }
}

#[test]
fn subnormal_and_finite_empty_boxes_do_not_create_unsafe_regions() {
    for size in [0.0, -0.0, -1.0, f32::from_bits(1), f32::MIN_POSITIVE] {
        let node = Element::new(Role::Group).into_node();
        let layout = Layout {
            root: LayoutBox {
                width: size,
                height: size,
                ..LayoutBox::default()
            },
            ..Layout::default()
        };
        let undo = |_: &Element, _: Rect| {
            Some([1.0, 0.0, -0.0, 1.0, f32::from_bits(1), -f32::from_bits(1)])
        };
        let (hit, region) = layout.hit_test_turned_certified(&node, 50.0, 50.0, &undo, VIEW);
        assert_eq!(hit, layout.hit_test_turned(&node, 50.0, 50.0, &undo));
        if let Some(region) = region {
            for (x, y) in [(1.0, 1.0), (50.0, 50.0), (399.0, 299.0)] {
                if region.contains(x, y) {
                    assert_eq!(layout.hit_test_turned(&node, x, y, &undo), hit);
                }
            }
        }
        let (_, tiny) = layout.hit_test_turned_certified(
            &node,
            f32::from_bits(1),
            f32::from_bits(1),
            &undo,
            Rect::new(0.0, 0.0, f32::MIN_POSITIVE, f32::MIN_POSITIVE),
        );
        assert!(tiny.is_none());
    }
}

#[test]
fn finite_old_translation_cannot_certify_a_new_overflow_before_cancellation() {
    let node = Element::new(Role::Group)
        .prop("overflow_example_root", true)
        .child(Element::new(Role::Group))
        .into_node();
    let root = LayoutBox {
        x: f32::MAX,
        width: 0.0,
        height: 100.0,
        flags: CONTAINS_OVERLAY,
        children: vec![LayoutBox {
            x: -f32::MAX / 4.0,
            width: f32::MAX / 2.0,
            height: 100.0,
            flags: IS_OVERLAY,
            ..LayoutBox::default()
        }]
        .into(),
        ..LayoutBox::default()
    };
    let layout = Layout {
        root,
        ..Layout::default()
    };
    let undo = |node: &Element, _: Rect| {
        node.props
            .get("overflow_example_root")
            .map(|_| [1.0, 0.0, 0.0, 1.0, f32::MAX, 0.0])
    };
    let original = layout.hit_test_turned(&node, 0.0, 50.0, &undo);
    assert_eq!(original, Some(vec![0]));
    let overflow_x = f32::from_bits((104 + 127) << 23);
    assert_ne!(
        layout.hit_test_turned(&node, overflow_x, 50.0, &undo),
        original
    );
    let (hit, region) = layout.hit_test_turned_certified(
        &node,
        0.0,
        50.0,
        &undo,
        Rect::new(-f32::MAX / 2.0, 0.0, f32::MAX, 100.0),
    );
    assert_eq!(hit, original);
    assert!(region.is_none());
}
