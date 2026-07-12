# Changelog

All notable user-visible changes will be recorded here. This project has not
yet made a public release, so breaking changes do not receive compatibility
shims or migration notes.

## Unreleased

Target version: `1.0.0-rc1`.

### Added

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
