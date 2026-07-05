# Changelog

All notable user-visible changes will be recorded here. This project has not
yet made a public release, so breaking changes do not receive compatibility
shims or migration notes.

## Unreleased

### Added

- A Guix-first local verification interface through `make`.
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
- Portable Alt bindings were replaced with direct nested sequences:
  `w 0`–`w 9` pull numbered windows, `f 0`–`f 9` select frames, and
  `f H/J/K/L` exchange windows. Nested maps now provide useful `?` help.
- Layouts, placement rules, and desktop dumps now use versioned, atomic
  persistence formats.

### Fixed

- End-to-end tests no longer inherit host keymap or compositor REPL state.
- Client disconnect, popup/grab, DnD, missing-output, and hotplug paths recover
  from stale protocol state instead of panicking.
- Local Guix builds no longer include workspace-only agent, cache, or runtime
  artifact directories in the source derivation.
- Pointer clamping now handles negative, vertical, gapped, and non-uniform
  output arrangements instead of assuming one horizontal strip.
