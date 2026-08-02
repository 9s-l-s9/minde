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
- VT switch away and back;
- clean exit to the console.

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
