// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        SwapBuffersError,
        renderer::{
            damage::{Error as DamageError, OutputDamageTracker},
            gles::{GlesError, GlesRenderer},
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::{
        EventLoop,
        ping::make_ping,
        timer::{TimeoutAction, Timer},
    },
    utils::{Clock, Monotonic, Rectangle, Transform},
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
        if std::mem::take(&mut state.winit_powered_off) {
            state.output_power_notify(output, true);
        }
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
                    state.absolute_pointer_motion.reset();
                    let mode = Mode {
                        size,
                        refresh: 60_000,
                    };
                    // The host size is the only realizable mode. Set its
                    // preference before change_current_state emits the
                    // wl_output mode flags; output-management retires the
                    // old resources when it observes the changed list.
                    for old in output.modes() {
                        if old != mode {
                            output.delete_mode(old);
                        }
                    }
                    output.set_preferred(mode);
                    output.change_current_state(Some(mode), None, None, None);
                    // Re-derive the usable area from the new size (layer
                    // exclusive zones re-arranged inside).
                    state.invalidate_lock_presentation();
                    state.observed_heads.clear();
                    state.update_usable_area();
                    // Re-send preferred fractional scales (surface/output
                    // association may have shifted with the geometry).
                    state.update_fractional_scales();
                    // Keep any lock surface covering the whole (resized) output.
                    state.reconfigure_lock_surfaces();
                    state.schedule_redraw();
                }
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Redraw => {
                    if let Err((stage, error)) = render_frame(
                        &mut backend, state, &output, &mut damage_tracker, &mut border_buffers,
                    ) {
                        // Rendering can advance the damage history before a
                        // failed swap. The replacement EGL surface needs a
                        // complete redraw, even if clients commit nothing else.
                        damage_tracker = OutputDamageTracker::from_output(&output);
                        if matches!(error, SwapBuffersError::ContextLost(_)) {
                            tracing::error!(stage, %error,
                                "graphics context lost; stopping nested compositor");
                            state.loop_signal.stop();
                        } else {
                            tracing::warn!(stage, %error, "temporary winit frame failure; retrying");
                            // Use the existing paced wakeup, not an immediate
                            // redraw loop when a driver keeps returning errors.
                            state.schedule_redraw();
                        }
                    }
                }
                WinitEvent::CloseRequested => {
                    state.loop_signal.stop();
                }
                _ => (),
            };
        })?;

    Ok(())
}

fn render_error(error: DamageError<GlesError>) -> SwapBuffersError {
    match error {
        DamageError::Rendering(error) => error.into(),
        DamageError::OutputNoMode(error) => SwapBuffersError::TemporaryFailure(Box::new(error)),
    }
}

/// Draw and submit one frame. Never acknowledge presentation or send frame
/// callbacks when binding, rendering or submission failed.
fn render_frame(
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    state: &mut MindeState,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    border_buffers: &mut BorderBuffers,
) -> Result<(), (&'static str, SwapBuffersError)> {
    if state.locked || !state.output_enabled(output) || state.winit_powered_off {
        // Locks take precedence over disabled/DPMS-off outputs. Draw only the
        // lock surface, or solid black until one exists. An unlocked disabled
        // output likewise paints black, without sending client frame callbacks.
        let damage = Rectangle::from_size(backend.window_size());
        let lock = if state.locked {
            state.lock_surface_for(output).cloned()
        } else {
            None
        };
        {
            let (renderer, mut framebuffer) = backend.bind().map_err(|error| ("bind", error))?;
            let elements: Vec<MindeRenderElements<GlesRenderer>> =
                lock.as_ref().map_or_else(Vec::new, |lock| {
                    smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                        &mut *renderer,
                        lock.wl_surface(),
                        (0, 0),
                        output.current_scale().fractional_scale(),
                        1.0,
                        smithay::backend::renderer::element::Kind::Unspecified,
                    )
                });
            damage_tracker
                .render_output(
                    &mut *renderer,
                    &mut framebuffer,
                    0,
                    &elements,
                    [0.0, 0.0, 0.0, 1.0],
                )
                .map_err(|error| ("render", render_error(error)))?;
        }
        backend
            .submit(Some(&[damage]))
            .map_err(|error| ("submit", error))?;
        // The nested host owns physical presentation. A successful swap is
        // our evidence that its window has replaced the previous desktop.
        if state.locked {
            state.lock_frame_presented(output, state.lock_generation());
            if let Some(lock) = lock {
                smithay::desktop::utils::send_frames_surface_tree(
                    lock.wl_surface(),
                    output,
                    state.start_time.elapsed(),
                    Some(Duration::ZERO),
                    |_, _| Some(output.clone()),
                );
            }
        }
        let _ = state.display_handle.flush_clients();
        return Ok(());
    }
    // Honor the output's fractional scale (wlr-randr --scale,
    // via wlr-output-management) so the nested scene renders
    // at the same density fractional-scale clients paint at.
    let fscale = output.current_scale().fractional_scale();
    let scale = smithay::utils::Scale::from(fscale);
    let Some(output_geo) = state.space.output_geometry(output) else {
        return Ok(());
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
        let (renderer, mut framebuffer) = backend.bind().map_err(|error| ("bind", error))?;
        let elements = crate::handlers::screencopy::output_scene_elements(
            &mut *renderer,
            output,
            output_geo,
            (0, 0).into(),
            scale,
            &state.space,
            None,
            state.message.as_ref(),
            &state.overlays,
            focus,
            state.border_color,
            border_buffers,
        );
        damage_tracker
            .render_output(
                &mut *renderer,
                &mut framebuffer,
                age,
                &elements,
                [0.1, 0.1, 0.1, 1.0],
            )
            .map_err(|error| ("render", render_error(error)))?
            .damage
            .cloned()
    };
    // Submit only what changed (tracked against the buffer
    // age); an unchanged scene skips the swap entirely.
    if let Some(damage) = damage {
        backend
            .submit(Some(&damage))
            .map_err(|error| ("submit", error))?;
    }

    // Satisfy any queued screen-capture frames for this output
    // by re-compositing the scene into their buffers (shm).
    if !state.pending_captures.is_empty() {
        let time = Clock::<Monotonic>::new().now();
        crate::handlers::screencopy::satisfy_output_captures(
            backend.renderer(),
            output,
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

    let now = state.start_time.elapsed();
    let frames = crate::frame_callbacks::FrameCallbacks::new(&state.space, output, |o| {
        state.output_can_render(o)
    });
    if frames.send_visible(output, now) {
        state.parked_frames.schedule(&state.handle, now);
    }

    // Layer surfaces need frame callbacks too, or clients
    // like fuzzel draw once and then never repaint.
    for layer in smithay::desktop::layer_map_for_output(output).layers() {
        layer.send_frame(
            output,
            state.start_time.elapsed(),
            Some(Duration::ZERO),
            |_, _| Some(output.clone()),
        )
    }

    state.space.refresh();
    state.popups.cleanup();
    let _ = state.display_handle.flush_clients();

    Ok(())
}
