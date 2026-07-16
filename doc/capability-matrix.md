# Capability and support matrix

This document records the release status of user-visible capabilities. It is
not a historical StumpWM command inventory: names and behavior are judged by
minde's documented 1.0 interfaces.

Legend: **supported** is a 1.0 commitment, **experimental** works but has an
incomplete release gate, **external** is deliberately delegated, and
**missing** needs implementation or an explicit pre-1.0 exclusion.

| Area | Status | Current verification / limitation |
|---|---|---|
| Manual frame trees, gaps, resize and layouts | supported | Scheme unit suites and nested e2e |
| Groups, per-head trees and dynamic master/stack groups | supported | Scheme unit suites and nested e2e |
| Floating windows and pointer move/resize | supported | Scheme unit suites and nested e2e |
| Window navigation, numbering, marks and placement rules | supported | Scheme unit suites and nested e2e |
| Native prompt, menu, command/eval and contextual help | supported | Scheme unit suites and nested e2e |
| Hooks and live Scheme configuration | supported | canonical API, atomic reload and serialized main-thread IPC |
| Native Wayland xdg-shell clients | supported | required foot gate plus optional GTK, Qt, browser, Emacs and SDL structured scenarios |
| Xwayland clients | experimental | required xterm mapping gate; post-crash Xwayland restart remains manual/missing |
| Layer shell and exclusive zones | experimental | bounded eww/fuzzel/swaybg scenarios; owner visual/exclusive-zone check remains |
| Session lock protocol | experimental | `ext-session-lock-v1` backs `logout!`/`lock-screen!`/`suspend!` ((minde session)); owner hardware verification that a locker actually blocks input/output remains |
| Clipboard between Wayland and X11 | experimental | required Wayland MIME round trip and compositor e2e; primary selection/DnD remain |
| Multiple outputs and hotplug | experimental | deterministic geometry properties, Scheme head tests and simulated soak cycle; hardware reports remain |
| DRM/libinput login session | experimental | used on X1/T450s; guided retained reports remain |
| Fullscreen, urgency and synthesized/remapped keys | supported | Scheme and nested e2e coverage |
| Session diagnostics and crash log | experimental | structured status/report e2e passes; real-session privacy inspection remains |
| Modeline and system tray | external | use eww through the schema-v1 query/subscription interface |
| Wallpaper and launcher | external | use layer-shell clients such as swaybg and fuzzel |
| Hard compositor restart preserving clients | external | unsupported by design; validated atomic config reload replaces it |
| Screencopy/screenshot protocol | experimental | `ext-image-copy-capture-v1` + `ext-image-capture-source-v1` (output sources); grim shm capture verified nested (winit), full-frame damage, no cursor session; udev/DRM path shares the handler but wants hardware verification |
| Input methods, touch/tablet and drag-and-drop | missing | best-effort unless promoted into the tested 1.0 matrix |
| Broad NVIDIA/multi-GPU support | missing | tested-hardware-only policy for 1.0 |

## StumpWM-equivalence boundary

The intended policy features are present, including manual and dynamic tiling,
groups, floating windows, placement, navigation, prompts, menus, help, hooks,
Xwayland, clipboard, and multiple heads. The mode line/tray is deliberately
not compositor policy. Feature equivalence does not imply complete coverage of
every modern Wayland protocol or GPU configuration; those commitments are
tracked separately above.

Update this table in the same change that promotes, removes, or deliberately
excludes a capability.
