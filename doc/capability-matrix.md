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
| Clipboard between Wayland and X11 | experimental | required Wayland MIME round trip and compositor e2e; primary selection (zwp_primary_selection, middle-click paste) now wired across Wayland and Xwayland; DnD remains |
| Clipboard managers (data-control) | experimental | `zwlr_data_control_manager_v1` and `ext_data_control_manager_v1` advertised (Smithay delegates), sharing the clipboard/primary selection; nested e2e asserts the globals plus wl-clipboard clipboard and primary round trips; cliphist-style manager on hardware remains |
| Foreign-toplevel management | experimental | hand-written `zwlr_foreign_toplevel_manager_v1` v3 mirrors the window registry (title, app_id, activated, outputs); activate/close/fullscreen routed through the group model, minimize a documented no-op; nested e2e binds the manager against a live toplevel, Rust/Scheme unit tests cover the request handlers; GUI taskbar on hardware remains |
| Output management (wlr-randr/kanshi) | experimental | hand-written `zwlr_output_manager_v1` v4; query plus apply of scale/position/transform reconciled into the `(wm-outputs)`/heads model, gated by the optional `output-configuration-allowed?` policy predicate; mode changes fail under winit (fixed host size); nested e2e queries and applies a scale via wlr-randr; udev mode-setting and kanshi on hardware remain |
| Fractional scaling and viewport | experimental | `wp-fractional-scale-v1` and `wp-viewporter` served on both backends via Smithay's delegates; each surface's preferred fractional scale follows its output and is re-sent when the output scale changes (wlr-randr `--scale 1.5`); render/screencopy paths use the output's f64 fractional scale (buffer sizes stay physical); nested e2e asserts both globals, applies a 1.5 scale and runs foot at it without protocol error; per-monitor mixed-scale tuning on hardware remains |
| Multiple outputs and hotplug | experimental | deterministic geometry properties, Scheme head tests and simulated soak cycle; hardware reports remain |
| DRM/libinput login session | experimental | used on X1/T450s; guided retained reports remain |
| Fullscreen, urgency and synthesized/remapped keys | supported | Scheme and nested e2e coverage |
| Session diagnostics and crash log | experimental | structured status/report e2e passes; real-session privacy inspection remains |
| Modeline and system tray | external | use eww through the schema-v1 query/subscription interface |
| Wallpaper and launcher | external | use layer-shell clients such as swaybg and fuzzel |
| Hard compositor restart preserving clients | external | unsupported by design; validated atomic config reload replaces it |
| Screencopy/screenshot protocol | experimental | `ext-image-copy-capture-v1` + `ext-image-capture-source-v1` (output sources) and legacy `wlr-screencopy-unstable-v1` v3 (wf-recorder, `xdg-desktop-portal-wlr`); both share one capture queue. grim (ext) and wf-recorder (wlr) shm capture verified nested (winit), full-frame damage, honest region capture; udev/DRM path and portal browser sharing want hardware verification |
| Pointer constraints and relative pointer | experimental | `zwp_pointer_constraints_v1` (lock/confine) and `zwp_relative_pointer_manager_v1` via Smithay's server modules; a locked pointer parks while relative motion still flows and the cursor-position hint is honored on unlock, a confined pointer clamps to its region, constraints activate on pointer focus; udev forwards true libinput deltas, winit synthesizes relative deltas from absolute motion; nested e2e asserts both globals plus Rust unit tests of the clamp logic; a real pointer-lock game on hardware remains |
| Idle notify and idle inhibit | experimental | `ext-idle-notify-v1` and `zwp_idle_inhibit_manager_v1` via Smithay's server modules; per-seat idle timers reset on every input event at the central `process_input_event` dispatch (keyboard/pointer/axis/touch), so a swayidle-style daemon can drive auto-lock/DPMS; an idle inhibitor on any surface suppresses idle notifications (fullscreen video/calls keep the screen awake) while any live inhibitor exists -- per-surface visibility is not tracked (documented in `handlers::idle`), and dead inhibitors are pruned so a crashed client cannot pin the screen awake; nested e2e asserts both globals and runs swayidle with a 1s timeout to confirm an `idled` notification actually fires; idle policy stays external (no Scheme surface, same stance as the locker) |
| Cursor shape and Xcursor theming | experimental | `wp_cursor_shape_manager_v1` (`cursor-shape-v1`) via Smithay's `cursor_shape` module on both backends; named cursors (the default pointer and every cursor-shape request) resolve through an Xcursor theme from `XCURSOR_THEME`/`XCURSOR_SIZE` (defaults `default`/24), scaled by the output's integer-ceil scale, honouring the image hotspot, and rendered through the shared `CursorState` path so themed/shape/client cursors all show on the udev pointer and in screen captures; falls back to the built-in bitmap when no theme is installed, animated cursors use their first frame only; nested e2e asserts the global and startup under an explicit theme/size, Rust unit tests cover shape-name and size selection; the udev on-screen themed pointer on real hardware remains owner-verified |
| Input methods (IME / on-screen keyboards) | experimental | `zwp_text_input_manager_v3` (`text-input-v3`) and `zwp_input_method_manager_v2` (`input-method-v2`) via Smithay's `text_input`/`input_method` modules on both backends; text-input focus follows keyboard focus (`set_text_input_focus`, from `focus_changed` and the focus-clear paths), the input-method candidate popup is tracked in the shared `PopupManager` and rendered at the text-input cursor rectangle, the input-method manager admits every client first-come-first-served (protocol enforces one active IME per seat; no Scheme surface, the IME is the user's session daemon). Nested e2e asserts both globals plus a live foot text-input-v3 client mapping/focusing with no protocol error; a full fcitx5/ibus round trip on hardware remains owner-verified |
| Virtual keyboard and pointer (automation/accessibility) | experimental | `zwp_virtual_keyboard_manager_v1` via Smithay's `virtual_keyboard` module (client filter admits every client -- these are user-session automation tools) and a hand-written `zwlr_virtual_pointer_manager_v1` v2 (Smithay ships no server module) on the wlr bindings. Injected keys reach the focused surface's `wl_keyboard` directly (honestly gated by focus, so they never leak past the session lock, whose focus is its lock surface); virtual pointer motion/button/axis run through the *same* `pointer_relative_motion`/`pointer_absolute_motion`/`pointer_button_event`/`pointer_axis_frame` helpers as real input, so pointer constraints, idle-activity reset and the session-lock gate all apply. Nested e2e asserts both globals, uses wtype to type into a focused foot client and asserts the text lands, and drives the virtual pointer with wlrctl; wtype/ydotool/accessibility tools on hardware remain |
| Keyboard shortcuts inhibit (remote-desktop/VM) | experimental | `zwp_keyboard_shortcuts_inhibit_manager_v1` via Smithay's `keyboard_shortcuts_inhibit` module. Inhibitors are auto-granted (no user prompt, documented) but active only while their surface holds keyboard focus; an active inhibitor makes `process_input_event` bypass the prefix-key grab and every Scheme shortcut (checked right after the session-lock gate, so it can never apply on the lock surface), and focus loss deactivates it immediately (`update_keyboard_shortcuts_inhibitors`, from `focus_changed` and every focus-clear path). Nested e2e asserts the global; a real remote-desktop/VM client on hardware remains |
| Touch/tablet and drag-and-drop | missing | best-effort unless promoted into the tested 1.0 matrix |
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
