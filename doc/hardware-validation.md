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
