# minde release status and roadmap

This document records the release decisions for the first public release,
the current implementation status, and the acceptance gates for 1.0.

## Release decisions

- Target: Guix-first 1.0.
- Compatibility: the unreleased API may break freely; no aliases or migration
  guide are required.
- Naming: descriptive Emacs/Scheme names for APIs and a simplified,
  Helix-inspired prefix keymap.
- Stable surface: documented Scheme modules, configuration, CLI, hooks,
  persisted formats, environment variables, and status/IPC schemas. Rust and
  low-level `wm-*` procedures remain internal.
- License: GPL-3.0-or-later for project-owned code, retaining the licenses and
  notices of Smithay/smallvil-derived code.
- Configuration: portable defaults live here; personal programs, scripts, and
  autostart configuration belong in the user's own configuration repository.
- Modeline and tray: deliberately external; minde provides a versioned
  status interface suitable for eww.
- Automation: local Guix checks are authoritative for 1.0. Hosted CI calls
  exactly the same noninteractive local entry point (`scripts/ci`); no gate
  logic lives only in the hosted configuration.
- Hardware: nested mode is the general test backend. DRM support is claimed
  only for explicitly tested hardware.

## Current status

Implemented and internally verified:

- Trustworthy baseline: reproducible pinned build environment, unit and
  nested end-to-end gates, licensing and provenance metadata, and a
  release-metadata check.
- Reusable Scheme foundations: `guile-minde-foundation` (geometry, trees,
  serialization, hooks, key notation) and `guile-minde-ui` (prompt/menu
  state machines), packaged independently through the Guix channel.
- Canonical API and atomic configuration reload: a failed reload preserves
  the live configuration.
- Runtime safety: client input cannot reach an unreviewed panic path;
  IPC stress and soak testing show no race, deadlock, stale grab, or
  unbounded growth in the exercised scenarios.
- Diagnostics and a stable, versioned IPC/status surface with rich error
  payloads, runtime API introspection (`describe-api`, drift-gated against
  `doc/generated/api-catalog.scm`), and a read-only event push socket
  (`minde-events.sock`, bounded fan-out, slow-consumer eviction, lock-time
  redaction, `mindectl subscribe --events`).
- Window management: frames, groups, dynamic groups, floats, placement
  rules, exposé/fselect, remapped keys, fullscreen, urgency, and the
  interactive help surface.
- Documentation and demonstrations: generated API reference and keybinding
  tables, an offline manual, and scripted demo clips with a drift-gated
  manifest.
- Guix packaging and local release automation: reproducible committed
  source archives, offline Cargo resolution, an archive-only Guix build,
  and `scripts/ci` producing byte-reproducible release artifacts.
- Ecosystem protocols: data-control (wlr+ext), primary selection including
  Xwayland, foreign-toplevel management, output management with a Scheme
  policy gate, pointer constraints and relative pointer, fractional scale
  and viewporter, idle-notify and idle-inhibit, cursor-shape with Xcursor
  themes, text-input-v3/input-method-v2, virtual keyboard/pointer,
  keyboard-shortcuts-inhibit (lock-gated, focus-scoped), presentation-time,
  tearing-control (advisory), linux-drm-syncobj (capability-guarded,
  udev-only), and the libinput configuration surface
  (`wm-input-devices`, `wm-configure-input!`).
- Screen capture: see the section below.
- Touch and tablet input: `wl_touch` on both backends with tap-to-focus,
  lock gating, and idle wiring; `zwp_tablet_manager_v2` with tool routing,
  pressure/tilt axes, and a pointer-emulation fallback for tablet-unaware
  clients.
- Machine-readable control interface: the IPC eval socket, the API catalog,
  data-only return values, self-describing error payloads, and the event
  stream together form a control surface for scripts and automation
  clients as well as interactive REPL use.

Pending owner verification on real hardware (nested equivalents pass):

- Physical reports for the reference laptops (T450s, X1), a long soak run,
  a release-candidate SDDM login, and rollback.
- Protocol behavior with canonical external clients: clipboard managers,
  kanshi/wlr-randr layout changes, pointer-lock clients, fractional-scale
  displays, swayidle, fcitx5, wtype, presentation-time/syncobj on DRM,
  and per-device libinput rules.
- Recording with `wf-recorder` and browser screen sharing through
  `xdg-desktop-portal-wlr` + PipeWire.
- Real touch and stylus behavior (tap, scroll, pressure, lock-leak check)
  on a touch laptop.

Known follow-ups:

- Make the IPC socket path overridable in `src/ipc.rs` instead of relying
  on runtime-directory isolation.
- Audit Scheme modules exporting `set!`-able configuration for the
  declarative-module pitfall fixed for session variables.
- Pending capture frames for an unplugged output linger until the client
  drops the session; no periodic `ImageCopyCaptureState::cleanup()` call.
- The one-shot 64KB IPC protocol ceiling is documented but not lifted.

## Screen capture and desktop portal

`ext-image-copy-capture-v1` + `ext-image-capture-source-v1` are wired on
both backends (`src/handlers/screencopy.rs`): shm capture everywhere,
dmabuf constraints offered on udev for a future zero-copy screen-cast
path; output capture only — no cursor sessions or toplevel sources are
advertised. Captures are queued and satisfied after a real composited
frame. `zwlr_screencopy_manager_v1` v3 is hand-implemented in
`src/handlers/wlr_screencopy.rs` (Smithay ships no server module) and
reuses the same machinery; region capture clips to the output. A bounded
gate (`tests/screencapture-e2e.sh`, part of `make check-e2e`) validates
format, dimensions against the queried output mode, and non-flat content,
and proves a bounded `wf-recorder` recording nested.

Portal status: `xdg-desktop-portal-wlr` bridges `wlr-screencopy` to
PipeWire for browser screen sharing, the same approach sway and river
use. The longer-term path is a portal backend speaking
`ext-image-copy-capture-v1`.

## 1.0 acceptance gates

- Every public Scheme export is documented and directly tested.
- No default key collision or undocumented binding exists.
- Rust formatting and fatal Clippy pass; client input cannot reach an
  unreviewed panic path.
- Unit/property, nested e2e, required applications, docs, and offline Guix
  package checks pass.
- Failed reloads preserve the live configuration.
- IPC stress/soak shows no race, deadlock, stale grab, or unbounded growth.
- X1 and T450s have retained hardware reports.
- SDDM starts the package built from release artifacts.
- License, provenance, versions, changelog, checksums, limitations, and
  rollback instructions are complete.
