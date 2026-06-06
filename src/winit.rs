use std::time::Duration;

use smithay::{
    backend::{
        renderer::{
            ImportAll, ImportMem,
            damage::OutputDamageTracker,
            element::{
                Kind,
                solid::{SolidColorBuffer, SolidColorRenderElement},
                surface::WaylandSurfaceRenderElement,
            },
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
};

// Space window surfaces plus our solid-color focus border.
smithay::backend::renderer::element::render_elements! {
    pub MindeRenderElements<R> where R: ImportAll + ImportMem;
    Surface=WaylandSurfaceRenderElement<R>,
    Solid=SolidColorRenderElement,
}

/// Focused-window border: gruvbox yellow, matching the user's StumpWM theme.
const BORDER_COLOR: [f32; 4] = [0.84, 0.60, 0.13, 1.0]; // #d79921
const BORDER_WIDTH: i32 = 2;

use crate::MindeState;
use crate::guile;

pub fn init_winit(
    event_loop: &mut EventLoop<MindeState>,
    state: &mut MindeState,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut backend, winit) = winit::init()?;

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Smithay".into(),
            model: "Winit".into(),
            serial_number: "Unknown".into(),
        },
    );
    let _global = output.create_global::<MindeState>(&state.display_handle);
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);

    state.space.map_output(&output, (0, 0));

    {
        let size = backend.window_size();
        guile::on_output_geometry(size.w.max(0) as u32, size.h.max(0) as u32);
    }

    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    // Persistent buffers for the 4 border edges (stable element ids keep
    // damage tracking incremental).
    let mut border_buffers: [SolidColorBuffer; 4] =
        std::array::from_fn(|_| SolidColorBuffer::new((0, 0), BORDER_COLOR));

    event_loop.handle().insert_source(winit, move |event, _, state| {
        match event {
            WinitEvent::Resized { size, .. } => {
                output.change_current_state(
                    Some(Mode {
                        size,
                        refresh: 60_000,
                    }),
                    None,
                    None,
                    None,
                );
                guile::on_output_geometry(size.w.max(0) as u32, size.h.max(0) as u32);
            }
            WinitEvent::Input(event) => state.process_input_event(event),
            WinitEvent::Redraw => {
                let size = backend.window_size();
                let damage = Rectangle::from_size(size);

                // Border elements around the focused window, if any.
                let mut custom: Vec<MindeRenderElements<GlesRenderer>> = Vec::new();
                if let Some(geo) = state
                    .focused_window
                    .as_ref()
                    .and_then(|w| state.space.element_geometry(w))
                {
                    let t = BORDER_WIDTH;
                    let (x, y, w, h) = (geo.loc.x, geo.loc.y, geo.size.w, geo.size.h);
                    let rects = [
                        ((x - t, y - t), (w + 2 * t, t)), // top
                        ((x - t, y + h), (w + 2 * t, t)), // bottom
                        ((x - t, y), (t, h)),             // left
                        ((x + w, y), (t, h)),             // right
                    ];
                    for (buf, (loc, sz)) in border_buffers.iter_mut().zip(rects) {
                        buf.update(sz, BORDER_COLOR);
                        custom.push(
                            SolidColorRenderElement::from_buffer(
                                buf,
                                smithay::utils::Point::<i32, smithay::utils::Logical>::from(loc)
                                    .to_physical(1),
                                1.0,
                                1.0,
                                Kind::Unspecified,
                            )
                            .into(),
                        );
                    }
                }

                {
                    let (renderer, mut framebuffer) = backend.bind().unwrap();
                    smithay::desktop::space::render_output::<
                        _,
                        MindeRenderElements<GlesRenderer>,
                        _,
                        _,
                    >(
                        &output,
                        renderer,
                        &mut framebuffer,
                        1.0,
                        0,
                        [&state.space],
                        &custom,
                        &mut damage_tracker,
                        [0.1, 0.1, 0.1, 1.0],
                    )
                    .unwrap();
                }
                backend.submit(Some(&[damage])).unwrap();

                state.space.elements().for_each(|window| {
                    window.send_frame(
                        &output,
                        state.start_time.elapsed(),
                        Some(Duration::ZERO),
                        |_, _| Some(output.clone()),
                    )
                });

                state.space.refresh();
                state.popups.cleanup();
                let _ = state.display_handle.flush_clients();

                // Ask for redraw to schedule new frame.
                backend.window().request_redraw();
            }
            WinitEvent::CloseRequested => {
                state.loop_signal.stop();
            }
            _ => (),
        };
    })?;

    Ok(())
}
