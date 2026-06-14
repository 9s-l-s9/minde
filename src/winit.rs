use std::time::Duration;

use smithay::{
    backend::{
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
};

use crate::MindeState;
use crate::guile;
use crate::render::{BorderBuffers, MindeRenderElements};

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

    // Announce the initial usable area (full output; no layers yet).
    state.update_usable_area();

    // Autostart hook: run once the first (and only, for winit) output is up.
    guile::on_startup();

    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    // Persistent buffers for the 4 border edges (stable element ids keep
    // damage tracking incremental).
    let mut border_buffers = BorderBuffers::default();

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
                // Re-derive the usable area from the new size (layer
                // exclusive zones re-arranged inside).
                state.usable_area = None;
                state.update_usable_area();
            }
            WinitEvent::Input(event) => state.process_input_event(event),
            WinitEvent::Redraw => {
                let size = backend.window_size();
                let damage = Rectangle::from_size(size);

                // Border elements around the selected frame (falling back
                // to the focused window before the first sync).
                let mut custom: Vec<MindeRenderElements<GlesRenderer>> = Vec::new();
                if let Some(geo) = state.focus_rect.or_else(|| {
                    state
                        .focused_window
                        .as_ref()
                        .and_then(|w| state.space.element_geometry(w))
                }) {
                    custom.extend(border_buffers.elements(geo, 1));
                }

                {
                    let (renderer, mut framebuffer) = backend.bind().unwrap();
                    // Message overlay, centered, above everything.
                    if let Some(msg) = state.message.as_ref() {
                        if let Some(elem) = crate::render::message_element(
                            &mut *renderer,
                            msg,
                            (size.w, size.h),
                            1,
                        ) {
                            custom.insert(0, elem);
                        }
                    }
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

                // Layer surfaces need frame callbacks too, or clients
                // like fuzzel draw once and then never repaint.
                for layer in smithay::desktop::layer_map_for_output(&output).layers() {
                    layer.send_frame(
                        &output,
                        state.start_time.elapsed(),
                        Some(Duration::ZERO),
                        |_, _| Some(output.clone()),
                    )
                }

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
