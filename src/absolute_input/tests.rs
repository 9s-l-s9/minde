// SPDX-License-Identifier: MIT
use super::*;
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::utils::Transform;

fn output(name: &str, size: (i32, i32)) -> Output {
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
            size: size.into(),
            refresh: 60_000,
        }),
        None,
        None,
        None,
    );
    output
}
fn rule(name: &str, target: Option<&str>) -> InputRule {
    InputRule {
        match_name: name.into(),
        map_to_output: target.map(str::to_owned),
        ..Default::default()
    }
}
fn center(
    mappings: &AbsoluteInputMappings,
    space: &Space<Window>,
    id: &str,
) -> Option<Point<f64, Logical>> {
    mappings.position(space, id, (0.5, 0.5).into())
}

#[test]
fn device_associations_and_ordered_overrides_beat_hotplug_order() {
    let external = output("HDMI-A-1", (1200, 800));
    let panel = output("eDP-1", (800, 600));
    let drawing = output("DP-1", (600, 400));
    let mut space = Space::default();
    space.map_output(&external, (0, 0));
    space.map_output(&drawing, (-600, -400));
    space.map_output(&panel, (1200, 0));
    let mut mappings = AbsoluteInputMappings::default();
    mappings.configure("event1".into(), "touchscreen", Some("HDMI-A-1".into()), &[]);
    mappings.configure("event2".into(), "pen", Some("eDP-1".into()), &[]);
    mappings.configure("event3".into(), "absolute pointer", None, &[]);
    assert_eq!(
        center(&mappings, &space, "event1"),
        Some((600.0, 400.0).into())
    );
    assert_eq!(
        center(&mappings, &space, "event2"),
        Some((1600.0, 300.0).into())
    );
    assert_eq!(
        center(&mappings, &space, "event3"),
        Some((1600.0, 300.0).into())
    );

    let mut rules = vec![
        rule("", Some("DP-1")),
        rule("pen", Some("HDMI-A-1")),
        rule("pe", None),
    ];
    mappings.configure("event2".into(), "pen", Some("eDP-1".into()), &rules);
    assert_eq!(
        center(&mappings, &space, "event2"),
        Some((600.0, 400.0).into())
    );
    rules.push(rule("pen", Some("")));
    mappings.configure("event2".into(), "pen", Some("eDP-1".into()), &rules);
    assert_eq!(
        center(&mappings, &space, "event2"),
        Some((1600.0, 300.0).into())
    );
    mappings.configure(
        "event1".into(),
        "touchscreen",
        Some("HDMI-A-1".into()),
        &rules,
    );
    assert_eq!(
        center(&mappings, &space, "event1"),
        Some((-300.0, -200.0).into())
    );
}

#[test]
fn output_and_device_lifecycle_re_resolve_the_mapping() {
    let external = output("HDMI-A-1", (1200, 800));
    let panel = output("DSI-1", (800, 600));
    let drawing = output("DP-1", (600, 400));
    let mut space = Space::default();
    space.map_output(&external, (0, 0));
    space.map_output(&panel, (1200, 0));
    space.map_output(&drawing, (-600, -400));
    let mut mappings = AbsoluteInputMappings::default();
    mappings.configure(
        "event1".into(),
        "pen",
        Some("HDMI-A-1".into()),
        &[rule("pen", Some("DP-1"))],
    );
    assert_eq!(
        center(&mappings, &space, "event1"),
        Some((-300.0, -200.0).into())
    );
    space.unmap_output(&drawing);
    assert_eq!(
        center(&mappings, &space, "event1"),
        Some((600.0, 400.0).into())
    );
    space.unmap_output(&external);
    assert_eq!(
        center(&mappings, &space, "event1"),
        Some((1600.0, 300.0).into())
    );
    space.unmap_output(&panel);
    assert_eq!(center(&mappings, &space, "event1"), None);
    // A reconnected connector can be a new Output object with the same name.
    let reconnected = output("DP-1", (1000, 500));
    space.map_output(&reconnected, (200, -500));
    assert_eq!(
        center(&mappings, &space, "event1"),
        Some((700.0, -250.0).into())
    );
    reconnected.change_current_state(None, None, Some(Scale::Integer(2)), None);
    space.map_output(&reconnected, (-500, 100));
    assert_eq!(
        center(&mappings, &space, "event1"),
        Some((-250.0, 225.0).into())
    );
    mappings.remove("event1");
    space.map_output(&panel, (1200, 0));
    mappings.configure(
        "event1".into(),
        "replacement touchscreen",
        Some("DSI-1".into()),
        &[],
    );
    assert_eq!(
        center(&mappings, &space, "event1"),
        Some((1600.0, 300.0).into())
    );
    // With no internal panel or association, connector order is stable.
    space.unmap_output(&panel);
    space.map_output(&external, (0, 0));
    assert_eq!(
        center(&mappings, &space, "event1"),
        Some((-250.0, 225.0).into())
    );
}

#[test]
fn physical_coordinates_follow_all_output_transforms_and_fractional_scale() {
    let panel = output("LVDS-1", (800, 600));
    let mut space = Space::default();
    space.map_output(&panel, (-300, 200));
    let mut mappings = AbsoluteInputMappings::default();
    mappings.configure("event1".into(), "touchscreen", None, &[]);
    for (transform, expected) in [
        (Transform::Normal, (-172.0, 344.0)),
        (Transform::_90, (36.0, 328.0)),
        (Transform::_180, (212.0, 536.0)),
        (Transform::_270, (-156.0, 712.0)),
        (Transform::Flipped, (212.0, 344.0)),
        (Transform::Flipped90, (-156.0, 328.0)),
        (Transform::Flipped180, (-172.0, 536.0)),
        (Transform::Flipped270, (36.0, 712.0)),
    ] {
        panel.change_current_state(None, Some(transform), Some(Scale::Fractional(1.25)), None);
        let point = mappings
            .position(&space, "event1", (0.2, 0.3).into())
            .unwrap();
        assert!(
            (point.x - expected.0).abs() < 1e-9 && (point.y - expected.1).abs() < 1e-9,
            "{transform:?}: {point:?} != {expected:?}"
        );
        for normalized in [(0.0, 0.0), (1.0, 1.0), (-2.0, 5.0)] {
            let point = mappings
                .position(&space, "event1", normalized.into())
                .unwrap();
            assert!(
                space
                    .output_geometry(&panel)
                    .unwrap()
                    .to_f64()
                    .contains(point)
            );
        }
    }
    for point in [(f64::NAN, 0.0), (0.0, f64::INFINITY)] {
        assert_eq!(mappings.position(&space, "event1", point.into()), None);
    }
}

#[test]
fn nested_input_ignores_the_framebuffer_flip() {
    let output = output("winit", (800, 600));
    output.change_current_state(None, Some(Transform::Flipped180), None, None);
    let mut space = Space::default();
    space.map_output(&output, (-1000, 200));
    let mappings = AbsoluteInputMappings::default();
    assert_eq!(
        mappings.position(&space, "winit", (0.2, 0.3).into()),
        Some((-840.0, 380.0).into())
    );
    assert_eq!(
        mappings.position(&space, "winit", (1.25, 0.5).into()),
        Some((0.0, 500.0).into())
    );
}
