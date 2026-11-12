// SPDX-License-Identifier: MIT
use super::*;
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};

fn output(name: &str, scale: f64) -> Output {
    let output = Output::new(
        name.into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "test".into(),
            model: "test".into(),
            serial_number: name.into(),
        },
    );
    output.change_current_state(
        Some(Mode {
            size: (1800, 1200).into(),
            refresh: 60_000,
        }),
        None,
        Some(Scale::Fractional(scale)),
        None,
    );
    output
}

fn scale_at(space: &Space<Window>, x: i32, width: i32) -> f64 {
    scale_for_geometry(
        space,
        Some(Rectangle::new((x, 100).into(), (width, 200).into())),
    )
}

#[test]
fn placement_and_return_use_current_geometry_without_space_refresh() {
    let mut space = Space::default();
    let left = output("left", 1.0);
    let right = output("right", 1.5);
    space.map_output(&left, (0, 0));
    space.map_output(&right, (1800, 0));
    for (x, expected) in [(100, 1.0), (1900, 1.5), (100, 1.0)] {
        assert_eq!(scale_at(&space, x, 300), expected);
    }
    assert_eq!(scale_for_geometry(&space, None), 1.0);
}

#[test]
fn resizing_a_spanning_window_changes_scale_by_overlap() {
    let mut space = Space::default();
    space.map_output(&output("left", 1.0), (0, 0));
    space.map_output(&output("right", 1.5), (1800, 0));
    assert_eq!(scale_at(&space, 1700, 150), 1.0);
    assert_eq!(scale_at(&space, 1700, 300), 1.5);
    assert_eq!(scale_at(&space, 1700, 200), 1.0); // Stable tie.
}

#[test]
fn output_reconfigure_disable_and_reenable_refresh_selection() {
    let mut space = Space::default();
    let left = output("left", 1.0);
    let right = output("right", 1.5);
    space.map_output(&left, (0, 0));
    space.map_output(&right, (1800, 0));
    right.change_current_state(None, None, Some(Scale::Fractional(2.0)), None);
    assert_eq!(scale_at(&space, 1900, 300), 2.0);
    space.unmap_output(&right);
    assert_eq!(scale_at(&space, 1900, 300), 1.0);
    space.map_output(&right, (-900, 0));
    assert_eq!(scale_at(&space, -800, 300), 2.0);
    space.unmap_output(&left);
    assert_eq!(scale_for_geometry(&space, None), 2.0);
    space.unmap_output(&right);
    assert_eq!(scale_at(&space, -800, 300), 1.0);
}
