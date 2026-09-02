// SPDX-License-Identifier: MIT

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
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
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

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| {
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
                    state.reported_heads.clear();
                    state.update_usable_area();
                    // Keep any lock surface covering the whole (resized) output.
                    state.reconfigure_lock_surfaces();
                }
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Redraw if state.locked => {
                    // Locked: render ONLY this output's lock surface, or solid
                    // black if it has not committed / the client died. Never
                    // the desktop -- this is the ext-session-lock guarantee.
                    let size = backend.window_size();
                    let damage = Rectangle::from_size(size);
                    {
                        let (renderer, mut framebuffer) = backend.bind().unwrap();
                        let mut elements: Vec<MindeRenderElements<GlesRenderer>> = Vec::new();
                        if let Some(lock) = state.lock_surface_for(&output) {
                            elements =
                                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                                    &mut *renderer,
                                    lock.wl_surface(),
                                    (0, 0),
                                    1.0,
                                    1.0,
                                    smithay::backend::renderer::element::Kind::Unspecified,
                                );
                        }
                        damage_tracker
                            .render_output(
                                &mut *renderer,
                                &mut framebuffer,
                                0,
                                &elements,
                                [0.0, 0.0, 0.0, 1.0],
                            )
                            .unwrap();
                    }
                    backend.submit(Some(&[damage])).unwrap();

                    // Frame callback so the lock client keeps drawing.
                    if let Some(lock) = state.lock_surface_for(&output) {
                        smithay::desktop::utils::send_frames_surface_tree(
                            lock.wl_surface(),
                            &output,
                            state.start_time.elapsed(),
                            Some(Duration::ZERO),
                            |_, _| Some(output.clone()),
                        );
                    }

                    let _ = state.display_handle.flush_clients();
                    backend.window().request_redraw();
                }
                WinitEvent::Redraw => {
                    // Honor the output's fractional scale (wlr-randr --scale,
                    // via wlr-output-management) so the nested scene renders
                    // at the same density fractional-scale clients paint at.
                    let fscale = output.current_scale().fractional_scale();
                    let scale = smithay::utils::Scale::from(fscale);
                    let Some(output_geo) = state.space.output_geometry(&output) else {
                        return;
                    };
                    let focus = state.focus_rect.or_else(|| {
                        state
                            .focused_window
                            .as_ref()
                            .and_then(|w| state.space.element_geometry(w))
                    });

                    // Same scene assembly as the udev backend and the capture
                    // paths. No cursor: the host compositor draws the pointer
                    // over the nested window.
                    let damage = {
                        let age = backend.buffer_age().unwrap_or(0);
                        let (renderer, mut framebuffer) = backend.bind().unwrap();
                        let elements = crate::handlers::screencopy::output_scene_elements(
                            &mut *renderer,
                            &output,
                            output_geo,
                            (0, 0).into(),
                            scale,
                            &state.space,
                            None,
                            state.message.as_ref(),
                            &state.overlays,
                            focus,
                            state.border_color,
                            &mut border_buffers,
                        );
                        damage_tracker
                            .render_output(
                                &mut *renderer,
                                &mut framebuffer,
                                age,
                                &elements,
                                [0.1, 0.1, 0.1, 1.0],
                            )
                            .unwrap()
                            .damage
                            .cloned()
                    };
                    // Submit only what changed (tracked against the buffer
                    // age); an unchanged scene skips the swap entirely.
                    if let Some(damage) = damage {
                        backend.submit(Some(&damage)).unwrap();
                    }

                    // Satisfy any queued screen-capture frames for this output
                    // by re-compositing the scene into their buffers (shm).
                    if !state.pending_captures.is_empty() {
                        let time = state.start_time.elapsed();
                        crate::handlers::screencopy::satisfy_output_captures(
                            backend.renderer(),
                            &output,
                            output_geo,
                            scale,
                            time,
                            &mut state.pending_captures,
                            &state.space,
                            &mut state.cursor_state,
                            state.pointer_location,
                            state.message.as_ref(),
                            &state.overlays,
                            focus,
                            state.border_color,
                        );
                        let _ = state.display_handle.flush_clients();
                    }

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
