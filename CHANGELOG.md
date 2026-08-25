# Changelog

All notable user-visible changes will be recorded here. This project has not
yet made a public release, so breaking changes do not receive compatibility
shims or migration notes.

## Unreleased

Target version: `1.0.0-rc1`.

### Added

- Automation primitives now provide paced, focus-pinned `wm-send-string` /
  `wm-type`, paced `wm-send-key` (including the `Enter` alias), symbolic and
  evdev pointer buttons with click counts, `wm-paste`, `wm-scroll`, pointer and
  window geometry inspection, and separate CLIPBOARD/PRIMARY setters. Native
  Wayland targets additionally support compositor-owned `wm-drop-files` and
  `wm-drop-text` copy operations with bounded token status history and
  `automation-result` events; file URI lists are validated, percent-encoded,
  multi-file capable, and CRLF terminated. XWayland drop targets explicitly
  report `unsupported-target` pending a separate XDND implementation.
- Clipboard ecosystem: primary selection
  (`zwp_primary_selection_device_manager_v1`) for middle-click paste, wired
  across Wayland clients and Xwayland (the X11 selection loop now mirrors the
  primary selection in both directions), plus clipboard-manager support via
  `zwlr_data_control_manager_v1` and `ext_data_control_manager_v1`. All three
  ride Smithay's selection delegates and share the compositor selection. A
  bounded nested e2e gate (`tests/clipboard-e2e.sh`, wired into
  `make check-e2e`) asserts the manager globals are advertised and that both
  the clipboard and the primary selection round-trip through wl-clipboard.
- `wlr-foreign-toplevel-management-unstable-v1` (v3, hand-written on the wlr
  bindings) so external bars, docks and switchers can enumerate windows and
  request activate/close/fullscreen/minimize. Per-window title, app-id,
  activated state and output association are kept in sync from the existing
  window-lifecycle and focus paths; activate/close/fullscreen route through
  the group/frame model (new Scheme hooks `handle-foreign-activate!`,
  `handle-foreign-fullscreen!`, `handle-foreign-minimize!` in
  `(minde groups)`), and minimize is a documented no-op (the tiling model
  has no minimized state). A bounded nested e2e gate
  (`tests/foreign-toplevel-e2e.sh`) binds the manager against a live toplevel.
- `wlr-output-management-unstable-v1` (v4, hand-written) so wlr-randr, kanshi
  and wdisplays can query and set the output layout (mode, position, scale,
  transform). Accepted configurations apply to the compositor outputs and
  reconcile back into the Scheme head model (`(wm-outputs)` /
  `handle-heads-change!`) exactly as a hotplug/resize does; external changes
  re-advertise to bound managers. Acceptance is gated by an optional Scheme
  policy predicate `output-configuration-allowed?` (default accept), and
  `handle-output-configured!` fires after an external change. Mode changes are
  refused under the winit backend (the host window fixes the size). A bounded
  nested e2e gate (`tests/output-management-e2e.sh`) queries the head and
  applies a scale via wlr-randr.
- Pointer constraints and relative pointer: `zwp_pointer_constraints_v1`
  (pointer lock and confinement) and `zwp_relative_pointer_manager_v1` (raw
  relative motion) via Smithay's server modules, for games and pointer-lock
  clients. A locked pointer stays parked while relative motion still flows, and
  the client's cursor-position hint is honored by warping the cursor there on
  unlock; a confined pointer is clamped to its region; constraints activate when
  the pointer enters their surface (focus-driven), per the protocol. Relative
  motion is forwarded from real libinput deltas on the udev backend and
  synthesized from consecutive absolute positions on the winit (nested) backend,
  so the relative-pointer protocol is usable in both. No Scheme surface: these
  are per-surface client requests, not compositor policy. A bounded nested e2e
  gate (`tests/pointer-constraints-e2e.sh`, wired into `make check-e2e`) asserts
  both manager globals are advertised; Rust unit tests cover the region-clamp
  and membership logic.
- Fractional scaling and viewport: `wp-fractional-scale-v1` and
  `wp-viewporter` served on both backends via Smithay's server modules.
  Clients learn each surface's preferred fractional scale, which follows the
  output the surface sits on and is re-sent to every mapped surface when an
  output's scale changes (e.g. wlr-randr `--scale 1.5`, through the
  output-management path). The render and screencopy paths now use the
  output's `f64` fractional scale throughout -- window content, the
  compositor chrome (focus border, message and frame-number overlays) and
  capture buffers -- replacing the previous integer-only assumptions; the
  winit backend renders at the output's fractional scale via its damage
  tracker, and screencopy buffer sizes stay physical (mode size), so a
  fractionally-scaled output is captured at full resolution. Viewporter is
  validated by Smithay's commit buffer handler and its surface render
  elements honor the destination size automatically. No new public Scheme
  exports: output scale (already settable through wlr-output-management)
  accepts fractional values. A bounded nested e2e gate
  (`tests/fractional-scale-e2e.sh`, wired into `make check-e2e`) asserts both
  globals via wayland-info, applies a 1.5 scale with wlr-randr and confirms
  the compositor reflects it and stays alive, then runs foot at that scale
  without a protocol error.
- Idle notify and idle inhibit: `ext-idle-notify-v1` and
  `zwp_idle_inhibit_manager_v1` (idle-inhibit) via Smithay's server modules,
  advertised on both backends. Per-seat idle timers reset on every input
  event at the central `process_input_event` dispatch, so a swayidle-style
  daemon can drive auto-lock, DPMS and dimming (idle policy stays external,
  the same stance `(minde session)` takes for the locker -- no Scheme
  surface is added). An idle inhibitor on any surface suppresses idle
  notifications so fullscreen video and calls keep the screen awake; idle is
  inhibited while any live inhibitor exists (per-surface visibility is not
  tracked, documented in `src/handlers/idle.rs`), and inhibitors whose client
  died are pruned so a crash cannot pin the screen awake. A bounded nested
  e2e gate (`tests/idle-e2e.sh`, wired into `make check-e2e`) asserts both
  globals via wayland-info and runs swayidle with a one-second timeout,
  confirming the compositor actually fires an idle notification.
- Cursor shape and Xcursor theming: `wp_cursor_shape_manager_v1`
  (`cursor-shape-v1`) via Smithay's `cursor_shape` module, advertised on both
  backends, so clients request a named cursor shape instead of attaching their
  own surface. Named cursors (the default pointer and every cursor-shape
  request) now resolve through an Xcursor theme loaded from `XCURSOR_THEME`
  and `XCURSOR_SIZE` (defaults: theme `default`, size 24), replacing the
  previously always-on hardcoded fallback bitmap; the requested cursor size
  scales with the output's integer-ceil scale so cursors stay crisp on HiDPI
  outputs. Themed images are cached per shape and scale, honour the image
  hotspot, and render through the same `CursorState` path as client-surface
  cursors -- so themed, shape-requested and client cursors all appear on the
  udev on-screen pointer and in screen captures. When no theme is installed
  (a bare environment), rendering falls back to the built-in bitmap. Animated
  cursors use their first frame only (documented in `src/render.rs`). Theme
  and size come from the environment per ecosystem convention, so no Scheme
  surface is added. A bounded nested e2e gate (`tests/cursor-shape-e2e.sh`,
  wired into `make check-e2e`) asserts the manager global via wayland-info and
  that the compositor stays stable under an explicit theme/size environment;
  Rust unit tests in `src/render.rs` cover shape-name resolution and integer
  size selection.
- Input methods via `text-input-v3` and `input-method-v2`:
  `zwp_text_input_manager_v3` and `zwp_input_method_manager_v2` advertised on
  both backends through Smithay's `text_input`/`input_method` modules, so an
  IME (fcitx5/ibus) or on-screen keyboard can drive preedit and commit into
  Wayland text fields. Text-input focus follows keyboard focus
  (`MindeState::set_text_input_focus`, driven from `SeatHandler::
  focus_changed` and the focus-clear paths), the input-method popup (candidate
  window) is tracked in the shared `PopupManager` and rendered at the
  text-input cursor rectangle by the existing per-window popup path, and the
  input-method manager admits every client first-come-first-served (the
  protocol enforces one active IME per seat; no Scheme surface, as the IME is
  the user's session daemon). A bounded nested e2e gate
  (`tests/text-input-e2e.sh`, wired into `make check-e2e`) asserts both manager
  globals via wayland-info and that a real text-input-v3 client (foot) maps,
  gains focus, and stays connected with no protocol error.
- Virtual keyboard and pointer: `zwp_virtual_keyboard_manager_v1`
  (Smithay's `virtual_keyboard` module, client filter admits every client --
  these are user-session automation and accessibility tools) and a hand-written
  `zwlr_virtual_pointer_manager_v1` v2 on the wlr bindings (Smithay ships no
  server module for it), for wtype/ydotool/wlrctl-style automation. Injected
  keys reach the focused surface's keyboard directly (gated by focus, so they
  never bypass the session lock); virtual pointer motion/button/axis are
  translated into the *same* pointer processing the real input path uses
  (factored into `pointer_relative_motion`/`pointer_absolute_motion`/
  `pointer_button_event`/`pointer_axis_frame` in `src/input.rs`), so pointer
  constraints, idle-activity reset and the locked-session gate all apply to
  virtual input. No Scheme surface.
- Keyboard shortcuts inhibit: `zwp_keyboard_shortcuts_inhibit_manager_v1`
  (Smithay's `keyboard_shortcuts_inhibit` module) for remote-desktop and VM
  clients. Inhibitors are auto-granted (documented; no user prompt) but active
  only while their surface has keyboard focus. An active inhibitor makes
  `process_input_event` bypass the prefix-key grab and every Scheme shortcut --
  checked right after the session-lock gate, so it can never take effect on the
  lock surface -- and focus loss deactivates it immediately
  (`update_keyboard_shortcuts_inhibitors`, called from `focus_changed` and
  every focus-clear path) so it can never survive focus loss. No Scheme surface.
- A bounded nested e2e gate (`tests/virtual-input-e2e.sh`, wired into
  `make check-e2e`) asserts all three manager globals, uses wtype to type a
  marker line into a focused foot client and asserts the text lands, and drives
  the hand-rolled virtual pointer with wlrctl while asserting the compositor
  stays alive and logs no protocol error.
- Presentation timing, tearing control and explicit sync:
  - `wp-presentation-time` (Smithay's `PresentationState`) is served
    **udev-only**. Each output's DRM frame now carries an
    `OutputPresentationFeedback` as its queue user-data; on the matching vblank
    it is delivered with the kernel's real monotonic timestamp, sequence and
    `Vsync|HwClock|HwCompletion` flags (falling back to the compositor clock and
    `Vsync` only when the driver reports no monotonic timestamps), the refresh
    interval from the output mode, and per-surface zero-copy flags taken from
    the render report. The nested winit backend has no true presentation clock,
    so it deliberately does not advertise the global.
  - `wp-tearing-control-v1` (`wp_tearing_control_manager_v1`, hand-written on
    the staging bindings since Smithay ships no server module) is advertised on
    **both** backends so tearing-aware clients find it. It is **advisory only**:
    the vendored Smithay `DrmCompositor` exposes no async/immediate page-flip
    path, so the `async` hint is recorded but never turned into a real tearing
    flip -- clients render identically either way. This is documented honestly
    in the capability matrix rather than claimed as working tearing.
  - `linux-drm-syncobj-v1` (`wp_linux_drm_syncobj_manager_v1`, Smithay's
    `DrmSyncobjState`) explicit sync is served **udev-only** and only when the
    primary GPU exposes syncobj timeline eventfds (`DRM_CAP_SYNCOBJ_TIMELINE`,
    probed by `supports_syncobj_eventfd`); hardware without support never gets
    the global. Smithay's renderer imports the client acquire fence as a KMS
    in-fence and signals the release point when the buffer is dropped, so no
    compositor-side commit changes were needed. No new Scheme surface.
  - A bounded nested e2e gate (`tests/presentation-tearing-e2e.sh`, wired into
    `make check-e2e`) asserts `wp_tearing_control_manager_v1` is advertised and
    that neither `wp_presentation` nor `wp_linux_drm_syncobj_manager_v1` appear
    on the winit backend (locking in the udev-only split); the udev presentation
    and syncobj paths are flagged for hardware-owner verification.
- libinput device configuration surface in Scheme: two compositor
  primitives, `(wm-input-devices)` (enumerate seat devices with their
  capability names) and
  `(wm-configure-input! match #:tap-to-click #:natural-scroll #:accel-profile
  #:click-method)`, plus an optional `handle-input-device-added!` hook. Rules
  are stored (deduplicated by match) and the udev/DRM backend applies them to
  every matching device on arrival (hotplug) and re-applies to devices already
  present when a rule changes at runtime; settings a device does not support
  are logged and skipped, never fatal. These are Rust-registered primitives,
  not part of the frozen `(minde …)` module API. The nested winit backend
  has no libinput, so `wm-input-devices` returns `()` and rules configure
  nothing. Covered by a Scheme unit test (`tests/input-config-test.scm`,
  argument validation and rule normalization on the no-libinput path) and a
  bounded nested e2e gate (`tests/input-config-e2e.sh`, wired into
  `make check-e2e`) that drives both primitives through `mindectl eval`;
  real per-device tap/scroll/accel/click on a touchpad is hardware
  owner-verification.
- Screen capture via `ext-image-copy-capture-v1` and
  `ext-image-capture-source-v1` (output capture sources). Screenshot
  tools such as grim can now grab a frame of an output on both the winit
  (nested) and udev/DRM backends. Capture buffers are filled after the
  output's on-screen frame is composited; shm is supported everywhere
  (grim's path, verified nested), and the udev backend additionally
  advertises dmabuf constraints from the primary render node for a future
  zero-copy screen-cast path. This first version reports full-frame
  damage and advertises only output sources -- no foreign-toplevel
  capture and no separate cursor session (cursors are drawn inline only
  when a client requests paint-cursors). A bounded nested e2e gate
  (`tests/screencapture-e2e.sh`, wired into `make check-e2e`) launches the
  nested compositor and asserts that `grim` captures a correctly-sized,
  non-blank PNG over `ext-image-copy-capture-v1`.
- Screen capture via legacy `wlr-screencopy-unstable-v1` (v3), the
  protocol wf-recorder and `xdg-desktop-portal-wlr` (browser screen
  sharing) speak. Served on both the winit and udev/DRM backends and
  advertised alongside the ext protocol; `capture_output`,
  `capture_output_region` (honest sub-rectangle capture), `copy` and
  `copy_with_damage` are all implemented, with shm everywhere and dmabuf
  advertised on udev. Both protocols share the single capture queue drained
  after each composite, so wlr frames reuse the ext scene-assembly and
  buffer-fill machinery rather than a parallel render loop. The nested e2e
  gate (`tests/screencapture-e2e.sh`) now additionally records a bounded
  `wf-recorder` clip and validates it (via `ffprobe` when available).
- `scripts/ci`, the single noninteractive CI entry point: the fast gate
  plus release-metadata by default, and `--release-artifacts` builds the
  deterministic archives twice, proves byte-reproducibility, recreates
  the crate mirror on a fresh checkout, and writes both archives plus
  `SHA256SUMS`. The hosted GitHub Actions workflow
  (`.github/workflows/ci.yml`) contains no gate logic: it installs Guix,
  restores the store cache, calls `scripts/ci`, and on version tags
  publishes the artifacts to a GitHub Release.
- A Guix channel (`.guix-channel`, `guix-channel/`) exposing
  `guile-minde-foundation` and `guile-minde-ui` from the committed
  tree. The compositor package joins the channel once the first tagged
  release publishes the vendored archive at a stable URL; the channel
  deliberately does not ship a package that cannot build from a
  `guix pull` checkout.
- `doc/generated/packaging.md`, a script-generated packaging reference
  (`scripts/generate-packaging-reference`): build/runtime dependencies,
  installed layout, session entry, archive verification, channel usage,
  and the first-party-Guix support boundary, drift-checked by
  `make check-docs`.
- `scripts/run-nested`, the documented nested-session entry point, now
  exists: it guards against a missing Guix shell, cargo, or graphical
  session, and runs the nested instance under an isolated
  `XDG_RUNTIME_DIR` so it cannot displace the IPC socket of a live outer
  Minde session.
- `doc/testing.md` is now a generated verification-command reference
  (`scripts/generate-testing-reference`), wired into `make docs` and
  drift-checked by `make check-docs`.
- `doc/debugging.md` documents interactive Scheme debugging
  (`MINDE_UNSAFE_REPL=1` REPL versus one-shot `mindectl eval`),
  Rust-side debugging (backtraces, gdb on the nested backend, the DRM
  seat caveat, crash.log), cold-build expectations, and editor setup;
  `CONTRIBUTING.md` carries condensed pointers.

### Fixed

- Overrides of `(minde session)` configuration variables
  (`%lock-command`, `%suspend-command`, `%lock-on-suspend?`,
  `%lock-timeout-ms`) were silently ignored in compiled builds because
  the module was declarative; the module is now non-declarative and
  `set!` from a personal configuration works as documented.
- `doc/support.md` no longer claims the session-lock protocol is
  missing.
- `tests/check-portable-defaults.sh` now fails loudly when ripgrep is
  absent instead of printing a false "ok"; `manifest.scm` gained
  `ripgrep` and `diffutils` so `./check` is self-contained in the
  project shell.

- Session management: `ext-session-lock-v1` compositor support (swaylock
  and other lockers can lock the session; while locked no desktop pixel
  is shown and no input reaches clients or keybindings), plus the
  `(minde session)` commands `lock-screen!`, `suspend!` (locks first
  and waits for lock confirmation, failing closed on timeout), and
  `logout!` (confirmation-gated), with `session-lock`/`session-unlock`
  hooks and the `wm-session-locked?` query.
- A Guix-first development environment plus a predictable fast `./check`,
  persistent nested runner, and explicit integration/release gates.
- Strict `--help` and `--version` output with an embedded build revision.
- Release, support, security, licensing, and provenance policies.
- Independently packaged `guile-minde-foundation` modules for geometry,
  trees, serialization, hooks, and key notation.
- Independently packaged, callback-driven `guile-minde-ui` prompt and menu
  engines that can be tested without a compositor or display.
- A canonical Scheme command registry, stable public module facades, and
  display-free configuration validation.
- A declarative default configuration and the `mindectl check-config`
  local validation command.
- A dependency-light portable C-t keymap, configurable terminal selection,
  and collision/personal-policy checks.
- A main-thread `mindectl eval` IPC endpoint and concurrent reload stress
  target; the separate-thread Guile REPL is now explicitly unsafe and opt-in.
- Schema-v1 `query state --json`, change-only `subscribe --json`, and atomic
  `status.json` interfaces for bars and monitoring.
- Human or newline-delimited JSON tracing, Scheme error backtraces, two-session
  log retention, and a conservatively redacted `mindectl report` bundle.
- Deterministic tree/geometry/serialization properties, Rust input/output and
  rendering tests, a structured application matrix, a reduced-keymap nested
  scenario, and a bounded RSS-reporting soak runner.
- Source-generated API and keymap inventories, a command/demo manifest, an
  offline HTML field manual, and deterministic WebM/poster/transcript capture
  for every registered command.
- Source-owned descriptions for all 274 bindings exported by the eight public
  Scheme modules, including explicit metadata for record accessors and values.
- Reproducible source and vendored-source archives, an archive-only public
  Guix package entry point, installed documentation/Guile modules, and a
  clean-tree local release pipeline with a self-contained bounded Guix
  environment, checksums, and release notes.
- A machine-readable 1.0 RC contract freezes public API/keymap inventories and
  configuration, status, persistence, and diagnostic schema versions.
- `make check-hardware` now writes a retained per-machine hardware snapshot and
  owner checklist; System/Home can opt into a vendored RC archive while keeping
  the checkout package as the default rollback path.

### Changed

- The project license metadata is consistently GPL-3.0-or-later while
  retaining the MIT terms for Smithay example-derived files.
- Mutable frame/group records now live in an explicitly internal compositor
  model module instead of being defined merely to connect public modules.
- Abbreviated StumpWM command and event names were replaced with descriptive
  kebab-case names; event handlers use the `handle-` prefix.
- Configuration reloads validate and build a candidate binding table before
  atomically publishing it.
- Keyboard layout, application launchers, locks, wallpaper, eww, and autostart
  moved out of repository defaults and into the personal Guix Home layer.
- Portable Alt and uppercase-letter bindings were replaced with lowercase
  nested sequences: `w h/j/k/l` move windows, `x h/j/k/l` exchange them,
  `w 0`–`w 9` pull numbered windows, and `f 0`–`f 9` select frames. Nested
  maps provide contextual `?` help.
- Directional exchange was flattened from `f x h/j/k/l` to `x h/j/k/l`; no
  portable binding now needs a second submap level for directional movement.
- Layouts, placement rules, and desktop dumps now use versioned, atomic
  persistence formats.

### Fixed

- The CLI contract check accepts semver pre-release versions
  (`1.0.0-rc1`), so `make check` passes on the release-candidate series.
- End-to-end tests no longer inherit host keymap or compositor REPL state.
- Client disconnect, popup/grab, DnD, missing-output, and hotplug paths recover
  from stale protocol state instead of panicking.
- Local Guix builds no longer include workspace-only agent, cache, or runtime
  artifact directories in the source derivation.
- Pointer clamping now handles negative, vertical, gapped, and non-uniform
  output arrangements instead of assuming one horizontal strip.
- The monolithic README is now an overview backed by focused tutorial,
  configuration, concepts, IPC/Eww, debugging, architecture, security,
  hardware, support, keymap, and demonstration guides.
- The end-to-end target now reports a missing `foot` dependency before entering
  the run-prompt scenario.
