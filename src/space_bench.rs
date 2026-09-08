// SPDX-License-Identifier: GPL-3.0-or-later
//! GPU-free checks and measurements of the Smithay Space membership walk
//! used for DRM presentation feedback. Elements model window identities and
//! geometry; timing excludes actual surface traversal and GPU work.

use smithay::{
    desktop::{Space, space::SpaceElement},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    utils::{IsAlive, Logical, Point, Rectangle, Transform},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Element(usize);

impl IsAlive for Element {
    fn alive(&self) -> bool {
        true
    }
}

impl SpaceElement for Element {
    fn bbox(&self) -> Rectangle<i32, Logical> {
        Rectangle::new((0, 0).into(), (100, 100).into())
    }
    fn is_in_input_region(&self, point: &Point<f64, Logical>) -> bool {
        self.bbox().to_f64().contains(*point)
    }
    fn set_activate(&self, _: bool) {}
    fn output_enter(&self, _: &Output, _: Rectangle<i32, Logical>) {}
    fn output_leave(&self, _: &Output) {}
}

fn scene(count: usize) -> (Space<Element>, [Output; 2]) {
    let mut space = Space::default();
    let outputs = [0, 1].map(|index| {
        let output = Output::new(
            format!("bench-{index}"),
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
                size: (200, 200).into(),
                refresh: 60_000,
            }),
            Some(Transform::Normal),
            None,
            None,
        );
        space.map_output(&output, (index * 200, 0));
        output
    });
    for id in 0..count {
        // First output, spanning both, second output, and parked offscreen.
        let x = [0, 150, 220, -10_000][id % 4];
        space.map_element(Element(id), (x, 0), false);
    }
    space.refresh();
    (space, outputs)
}

fn legacy_members(space: &Space<Element>, output: &Output) -> Vec<usize> {
    space
        .elements()
        .filter(|window| space.outputs_for_element(window).contains(output))
        .map(|window| window.0)
        .collect()
}

fn assert_same_members(space: &Space<Element>, outputs: &[Output]) {
    for output in outputs {
        assert_eq!(
            legacy_members(space, output),
            space
                .elements_for_output(output)
                .map(|window| window.0)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn output_iterator_preserves_membership_and_order_through_scene_changes() {
    let (mut space, outputs) = scene(12);
    assert_eq!(legacy_members(&space, &outputs[0]), vec![0, 1, 4, 5, 8, 9]);
    assert_eq!(legacy_members(&space, &outputs[1]), vec![1, 2, 5, 6, 9, 10]);
    assert_same_members(&space, &outputs);
    space.map_element(Element(0), (220, 0), false);
    space.raise_element(&Element(5), false);
    // Both APIs read the same cached membership, even before refresh.
    assert_same_members(&space, &outputs);
    space.refresh();
    assert_same_members(&space, &outputs);
    space.unmap_elem(&Element(1));
    space.unmap_output(&outputs[0]);
    space.refresh();
    assert_same_members(&space, &outputs);
    assert!(space.elements_for_output(&outputs[0]).next().is_none());
}

#[test]
#[ignore = "measurement only; run with --ignored --nocapture"]
fn bench_output_membership() {
    use std::{hint::black_box, time::Instant};
    for count in [12, 100, 1000] {
        let (space, outputs) = scene(count);
        let iterations = 5000;
        for legacy in [true, false] {
            let start = Instant::now();
            for _ in 0..iterations {
                for output in &outputs {
                    let space = black_box(&space);
                    let selected = if legacy {
                        space
                            .elements()
                            .filter(|window| space.outputs_for_element(window).contains(output))
                            .count()
                    } else {
                        space.elements_for_output(output).count()
                    };
                    black_box(selected);
                }
            }
            println!(
                "{count} windows, two outputs, {}: {:.3} us/pass",
                if legacy { "legacy" } else { "iterator" },
                start.elapsed().as_secs_f64() * 1e6 / iterations as f64
            );
        }
    }
}

#[test]
#[ignore = "measurement only; run with --ignored --nocapture"]
fn bench_frame_callback_selection() {
    use std::{hint::black_box, time::Instant};
    fn selected(space: &Space<Element>, output: &Output, short_circuit: bool) -> Vec<usize> {
        let output_geo = space.output_geometry(output).unwrap();
        let first = space.outputs().next() == Some(output);
        space
            .elements()
            .filter(|window| {
                let geo = space.element_geometry(window);
                let on_this = geo.map(|g| g.overlaps(output_geo)).unwrap_or(false);
                let parked = (!short_circuit || (first && !on_this))
                    && geo
                        .map(|g| {
                            !space
                                .outputs()
                                .filter_map(|o| space.output_geometry(o))
                                .any(|og| g.overlaps(og))
                        })
                        .unwrap_or(true);
                on_this || (first && parked)
            })
            .map(|window| window.0)
            .collect()
    }
    for count in [12, 100, 1000] {
        let (space, outputs) = scene(count);
        for output in &outputs {
            assert_eq!(
                selected(&space, output, false),
                selected(&space, output, true)
            );
        }
        for short_circuit in [false, true] {
            let start = Instant::now();
            let iterations = 1000;
            for _ in 0..iterations {
                for output in &outputs {
                    black_box(selected(black_box(&space), output, short_circuit));
                }
            }
            println!(
                "{count} windows, two outputs, {}: {:.3} us/pass",
                if short_circuit {
                    "short-circuit"
                } else {
                    "legacy"
                },
                start.elapsed().as_secs_f64() * 1e6 / iterations as f64
            );
        }
    }
}
