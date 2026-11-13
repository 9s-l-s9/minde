# Hardware and login-session validation

Automated nested tests cannot verify DRM modesetting, libinput, VT switching,
GPU drivers, suspend/resume, physical hotplug, or the display manager. Perform
this checklist from a spare text VT before removing the StumpWM rollback
session.

Create the retained, machine-specific report first:

```sh
make check-hardware
```

This only reads system information and writes a Markdown checklist under
`build/hardware/`; it does not reconfigure, log out, suspend, or change the
running session. Run it once on X1 and once on T450s, then fill in each result
while following the sections below.

## Preflight

```sh
make check
make check-e2e
make check-package
```

Ensure no unsaved work depends on the graphical session. Keep another VT and a
known-good StumpWM login entry available.

## Direct TTY run

From Ctrl+Alt+F3:

```sh
cd ~/Projects/minde
guix shell -m manifest.scm -- sh -c \
  'XKB_DEFAULT_LAYOUT=de XKB_DEFAULT_VARIANT=bone \
   cargo run --release --locked -- --tty'
```

Validate:

- cursor movement, buttons and keyboard layout;
- terminal launch and every accepted prefix submap;
- frame borders, split resizing, fullscreen and floating grabs;
- brightness/audio keys;
- native Wayland and Xterm/Xwayland windows;
- clipboard in both directions;
- output arrangement and physical hotplug when available;
- `wlr-randr` lists every (non-interlaced) connector mode with the preferred
  one flagged, make/model/serial are populated from EDID, and the head
  description reads `Make Model Serial` (connector name if no EDID);
- VT switch away and back;
- clean exit to the console.

DRM device ownership (udev, requires a removable GPU):

- Keep the primary GPU connected and repeatedly attach/remove a secondary
  GPU. Compare the compositor's `/proc/PID/fd` links to `/dev/dri/` before
  attachment and after each removal. Removed-device descriptors must not
  accumulate, and outputs from other devices must remain usable.
- Retain logs for a device that fails initialization. Repeated attempts must
  release the libseat device each time and leave no notifier or renderer
  registration behind.
- In a separate disposable session, removing the primary GPU must withdraw
  `wp_linux_drm_syncobj_manager_v1` from `wayland-info` when that interface was
  present. Removing a secondary GPU must preserve it. Primary-GPU replacement
  and driver recovery still require their own hardware validation.

Automated ownership tests exercise real file descriptors, failed DRM
initialization and calloop registration lifetimes without acquiring a seat.
They do not exercise a physical GPU's EGL/GBM teardown or libseat backend.

Repaint recovery (udev):

- With two monitors, park an animating window outside every output, then
  power off the first output. The hidden client's `wl_surface.frame` callbacks
  must continue at the hidden-window rate through the other output. A window
  confined to the powered-off monitor must stop receiving callbacks until that
  monitor wakes.
- With an animating client, change mode or adaptive sync while frames are in
  flight. Animation must continue without requiring pointer motion or another
  unrelated event to restart repainting.
- Switch away from Minde's VT and request an output configuration change over
  IPC. There must be no recurring render attempts during the pause. On return,
  the desktop must repaint with the latest configuration.
- If a driver failure can be reproduced, retain the debug log. Persistent
  render failures must slow to one attempt per second, with at most one render
  failure diagnostic per output every ten seconds. Once the device recovers,
  repainting must resume without requiring a new client commit.

Absolute input mapping (udev, requires a touchscreen or tablet and two outputs):

- Connect the external display before starting the compositor. Verify that
  touches land on the internal panel, or on the connector named by the device's
  `WL_OUTPUT` property, regardless of output discovery order.
- Set `#:map-to-output` with `wm-configure-input!` for that device, then use
  `'auto` to restore its automatic association. Check an already-present device
  and a device connected after the rule was stored.
- Rotate and scale the target output and move it to a negative layout origin.
  Check corners and an asymmetric interior point with both touch and stylus;
  confirm the visible cursor and the client's input agree.
- Disable/remove the target and verify fallback to an available associated
  output or internal panel. Re-enable/reconnect it and verify that mapping
  returns to the requested connector without reloading the rule.

Touch cancellation (udev, requires a touchscreen):

- Hold one or more fingers on a touch-capable client, including a stationary
  finger while another moves. Trigger a device cancellation, such as palm
  rejection or disabling the touch device, and confirm the client's
  `WAYLAND_DEBUG=1` trace receives `wl_touch.cancel` and its gesture ends.
- Repeat while locking, unmapping the touched window, and switching away
  from the VT. After recovery, a new touch must start a new gesture on the
  currently touched client; releasing an old contact must not revive it.

Enabling and disabling heads (udev, needs two monitors):

- `wlr-randr --output DP-1 --off`: the monitor goes to standby, `wlr-randr`
  still lists the head with `Enabled: no` and its full mode list, and every
  window that lived on it moves to a remaining head;
- `wlr-randr --output DP-1 --on`: the monitor comes back at its preferred
  mode, placed right of the other heads, and frames restore per head;
- unplug the monitor while it is disabled: the head disappears from
  `wlr-randr`; plug it back in: it reappears enabled at its preferred mode;
- VT switch away and back while a head is disabled: it stays off;
- a shikane profile with `enable = false` for `eDP-1` (lid-closed profile)
  turns the panel off and windows follow to the external head.

Scheme idle time (udev):

- Observe `(idle-ms)` from another terminal without sending compositor input;
  it increases with elapsed time. After switching away from the VT and back,
  it starts a fresh interval.
- Suspend and resume with an idle policy enabled. Before keyboard or pointer
  input, the value measures time since session activation and must not include
  the suspended interval or immediately trigger another suspend.

Output power management (udev; `wlopm` and `swayidle` are in `manifest.scm`):

- `wlopm --off '*'`: every monitor blanks (DPMS standby), `wlopm` reports
  `off`, and `wlr-randr` still lists each head with `Enabled: yes` -- windows
  and the layout are untouched;
- move the mouse or press a key: every monitor lights up again and `wlopm`
  reports `on` (wake on input);
- `swayidle -w timeout 5 'wlopm --off *' resume 'wlopm --on *'`: after 5 s
  idle the screens blank, activity brings them back, and repeating the cycle
  a few times leaves no stuck-black or duplicated-repaint output (watch the
  log for a single "powering head on" per head per cycle);
- VT switch away while a head is off and back: the head stays off until
  input or `wlopm --on`;
- unplug a monitor while it is off: it disappears from `wlopm`/`wlr-randr`
  and a running `wlopm` gets `failed`; plug it back in: it comes up on;
- `wlopm --off eDP-1` on a disabled head (`wlr-randr --output eDP-1 --off`
  first) fails rather than blanking a head that is not lit.
Mode setting (udev):

- `wlr-randr --output DP-1 --mode 1920x1080@60`: the monitor's OSD reports
  the new mode (scanout really changed, not just the advertised size),
  `wlr-randr` shows it as current, windows reflow to the new size and the
  pointer stays on screen;
- switch back to the preferred mode; then `--custom-mode 1920x1080@59.94`
  lands on the connector's 59.94 Hz mode (size equal, refresh within 1 Hz);
- an impossible custom mode (`--custom-mode 1234x567@60`) fails
  (`wlr-randr` reports failure) and nothing changes on the monitor;
- two heads in one configuration where one requests a bad mode
  (`--output DP-1 --mode 1920x1080@60 --output eDP-1 --custom-mode 1x1@1`):
  the whole configuration fails and *both* heads keep their previous mode;
- `--scale 2` on a HiDPI head re-renders fractional-scale clients (foot,
  GTK4) at the new density.

Adaptive sync (udev, needs a FreeSync/G-Sync-compatible monitor):

- `cat /sys/class/drm/card*-DP-1/vrr_capable` prints `1`;
- `wlr-randr --output DP-1 --adaptive-sync enabled` succeeds and
  `wlr-randr` then reports `Adaptive Sync: enabled`; `--adaptive-sync
  disabled` reports `disabled` again;
- on a head whose `vrr_capable` is `0`, `--adaptive-sync enabled` fails
  and the head still reports `Adaptive Sync: disabled`;
- `--off` then `--on` the VRR head: `Adaptive Sync` comes back as it was.

If the compositor wedges, switch VTs and terminate it from the spare console.
Do not reconfigure the display manager merely to debug a direct TTY failure.

## Login-session validation

After System/Home dry-runs and reviewing the changelog, apply the intended
machine configuration, inspect the generated files/store package, then log out
and select Minde. Confirm personal Eww, wallpaper, brightness policy,
autostart and Print-prefix behavior separately from repository defaults.

No reboot is normally required. Keep StumpWM selectable until the 1.0 hardware
gate passes on both X1 and T450s.

## Evidence

Run `./debug-tty.sh` when retained diagnostics are needed. Record machine,
kernel/Guix generation, GPU, connectors, input devices, suspend/hotplug result,
and relevant redacted logs. Hardware-specific failures must not be generalized
as portable behavior without reproducing them in the nested backend.

The release remains pending until both generated reports say `Outcome: pass`,
identify their retained diagnostic paths, and record rollback to the preceding
Guix generation.
