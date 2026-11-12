// SPDX-License-Identifier: MIT
use super::*;
use smithay::{
    desktop::Space,
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::wayland_protocols::xdg::shell::server::xdg_positioner::{
        Anchor, ConstraintAdjustment, Gravity,
    },
    utils::Transform,
};

fn rect(x: i32, y: i32, width: i32, height: i32) -> Rectangle<i32, Logical> {
    Rectangle::new((x, y).into(), (width, height).into())
}

#[test]
fn popup_uses_parent_output_and_nested_parent_coordinates() {
    let outputs = [rect(0, 0, 1920, 1080), rect(1920, 0, 1920, 1080)];
    let root = rect(3500, 800, 300, 200);
    let target = popup_constraint_rect(root, (0, 0).into(), outputs).unwrap();
    assert_eq!(target, rect(-1580, -800, 1920, 1080));
    let positioner = PositionerState {
        rect_size: (160, 120).into(),
        anchor_rect: rect(280, 180, 10, 10),
        anchor_edges: Anchor::BottomRight,
        gravity: Gravity::BottomRight,
        constraint_adjustment: ConstraintAdjustment::FlipX | ConstraintAdjustment::FlipY,
        ..Default::default()
    };
    assert_eq!(
        positioner.get_unconstrained_geometry(target),
        rect(120, 60, 160, 120)
    );
    assert_eq!(
        popup_constraint_rect(root, (120, 60).into(), outputs),
        Some(rect(-1700, -860, 1920, 1080))
    );
}

#[test]
fn spanning_popup_parent_uses_largest_overlap_with_stable_ties() {
    let outputs = [rect(0, 0, 1920, 1080), rect(1920, 0, 1920, 1080)];
    assert_eq!(
        popup_constraint_rect(rect(1850, 100, 400, 300), (0, 0).into(), outputs),
        Some(rect(70, -100, 1920, 1080))
    );
    assert_eq!(
        popup_constraint_rect(rect(1720, 100, 400, 300), (0, 0).into(), outputs),
        Some(rect(-1720, -100, 1920, 1080))
    );
}

#[test]
fn popup_bounds_use_scaled_rotated_output_in_space_coordinates() {
    let output = Output::new(
        "rotated".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "test".into(),
            model: "test".into(),
            serial_number: "test".into(),
        },
    );
    output.change_current_state(
        Some(Mode {
            size: (1800, 1200).into(),
            refresh: 60_000,
        }),
        Some(Transform::_90),
        Some(Scale::Fractional(1.5)),
        None,
    );
    let mut space = Space::<Window>::default();
    space.map_output(&output, (-800, -400));
    let outputs = space.outputs().filter_map(|o| space.output_geometry(o));
    assert_eq!(
        popup_constraint_rect(rect(-250, 600, 200, 150), (20, 30).into(), outputs),
        Some(rect(-570, -1030, 800, 1200))
    );
}

#[test]
fn popup_without_an_overlapping_output_uses_only_available_bounds() {
    let root = rect(3000, 4000, 200, 200);
    assert_eq!(popup_constraint_rect(root, (0, 0).into(), []), None);
    assert_eq!(
        popup_constraint_rect(root, (0, 0).into(), [rect(0, 0, 1280, 720)]),
        Some(rect(-3000, -4000, 1280, 720))
    );
}
