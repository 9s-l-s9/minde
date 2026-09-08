// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::{
        EventLoop,
        ping::make_ping,
        timer::{TimeoutAction, Timer},
    },
    utils::{Rectangle, Transform},
};

use crate::MindeState;
use crate::guile;
use crate::handlers::output_management::HeadChange;
use crate::render::{BorderBuffers, MindeRenderElements};

/// Backend hook for `MindeState::apply_output_configuration` under winit.
/// The host window fixes the output size, so a differing mode is refused;
/// enable/disable needs nothing backend-side (the generic caller unmaps
/// the output from the space and the redraw paints black); adaptive sync
/// is never supported (already rejected by validation).
pub fn realize_head(
    state: &mut MindeState,
    output: &Output,
    change: &HeadChange,
) -> Result<(), String> {
    if let Some(mode) = change.requested_mode()
        && Some(mode) != output.current_mode()
    {
        return Err("mode changes are not supported on the winit backend".into());
    }
    if change.adaptive_sync == Some(true) {
        return Err("adaptive sync is not supported on the winit backend".into());
    }
    if !change.enabled {
        // Like udev's disable_head: DPMS state is forgotten on disable so
        // a re-enabled head always lights up.
        state.winit_powered_off = false;
    }
    state.schedule_redraw();
    Ok(())
}

pub fn init_winit(
    event_loop: &mut EventLoop<MindeState>,
    state: &mut MindeState,
) -> Result<(), Box<dyn std::error::Error>> {
    let (backend, winit) = winit::init()?;

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
    state.output_management_add_output(&output);

    // Announce the initial usable area (full output; no layers yet).
    state.update_usable_area();

    // Autostart hook: run once the first (and only, for winit) output is up.
    guile::on_startup();

    // Scene changes wake the backend through calloop, without borrowing the
    // renderer inside an input/commit callback. A pending redraw coalesces
    // bursts; pacing also bounds clients that commit undamaged frames.
    let backend = Rc::new(RefCell::new(backend));
    let pending = Rc::new(Cell::new(false));
    let last_redraw = Rc::new(Cell::new(Instant::now()));
    let interval = Duration::from_nanos(1_000_000_000 / 60);
    let (ping, source) = make_ping()?;
    state.winit_redraw_ping = Some(ping);
    let redraw_backend = backend.clone();
    let redraw_pending = pending.clone();
    let redraw_time = last_redraw.clone();
    event_loop
        .handle()
        .insert_source(source, move |_, _, state| {
            if redraw_pending.replace(true) {
                return;
            }
            let deadline = redraw_time.get() + interval;
            if deadline <= Instant::now() {
                redraw_backend.borrow().window().request_redraw();
            } else {
                let backend = redraw_backend.clone();
                if state
                    .handle
                    .insert_source(Timer::from_deadline(deadline), move |_, _, _| {
                        backend.borrow().window().request_redraw();
                        TimeoutAction::Drop
                    })
                    .is_err()
                {
                    // Keep a failed timer registration from stranding the scene.
                    redraw_backend.borrow().window().request_redraw();
                }
            }
        })?;

    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    // Persistent buffers for the 4 border edges (stable element ids keep
    // damage tracking incremental).
    let mut border_buffers = BorderBuffers::default();

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| {
            let _render_measurement = matches!(&event, WinitEvent::Redraw)
                .then(|| crate::timing::Measurement::start(crate::timing::Probe::Render));
            let mut backend = backend.borrow_mut();
            if matches!(&event, WinitEvent::Redraw) {
                pending.set(false);
                last_redraw.set(Instant::now());
            }
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
                    // Re-send preferred fractional scales (surface/output
                    // association may have shifted with the geometry).
                    state.update_fractional_scales();
                    // Keep any lock surface covering the whole (resized) output.
                    state.reconfigure_lock_surfaces();
                    state.schedule_redraw();
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
                }
                WinitEvent::Redraw if !state.output_enabled(&output) || state.winit_powered_off => {
                    // Disabled via wlr-output-management, or DPMS off via
                    // wlr-output-power-management: the head stays
                    // advertised but shows nothing -- paint black, no
                    // elements, no frame callbacks. Power-on explicitly
                    // schedules the next redraw.
                    let size = backend.window_size();
                    let damage = Rectangle::from_size(size);
                    {
                        let (renderer, mut framebuffer) = backend.bind().unwrap();
                        let elements: Vec<MindeRenderElements<GlesRenderer>> = Vec::new();
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
                    let _ = state.display_handle.flush_clients();
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

                }
                WinitEvent::CloseRequested => {
                    state.loop_signal.stop();
                }
                _ => (),
            };
        })?;

    Ok(())
}
