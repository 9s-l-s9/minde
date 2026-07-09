# minde 1.0 release roadmap

This is the implementation and acceptance plan for the first public release.
Sprints are sequential: a sprint is complete only when its automated checks
pass and its manual verification has either passed or been explicitly recorded
as unavailable on the current hardware.

## Progress

- Sprint 0: implemented and internally verified; owner verification pending.
- Sprint 1: implemented and internally verified; owner verification pending.
- Sprint 2: implemented and internally verified; owner verification pending.
- Sprint 3: implemented and internally verified; owner verification pending.
- Sprint 4: implemented and internally verified; owner verification pending.
- Sprint 5: implemented and internally verified; owner verification pending.
- Sprint 6: implemented and internally verified; owner verification pending.
- Sprint 7: implemented and internally verified; owner verification and
  heavyweight optional application shards pending.
- Sprint 8: implemented and internally verified; generated references, split
  guides, the offline manual, complete live-export descriptions, and all
  scripted command media pass. Owner manual/video review remains.
- Sprint 9: implemented and internally verified; committed source archives are
  reproducible, offline Cargo resolution and the archive-only Guix build pass,
  installed contents are complete, and all 16 scripted videos were regenerated
  and validated. Owner clean-checkout release run and live SDDM login remain.
- Sprint 10: pending.

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
- Configuration: portable defaults live here. Personal programs, Print/de-bone
  preferences, scripts, eww configuration, and autostart live in
  `~/Projects/System`.
- Modeline and tray: deliberately external; minde provides a versioned
  status interface suitable for eww.
- Automation: local Guix checks are authoritative for 1.0. Hosted CI is
  deferred, but must eventually call the same noninteractive targets.
- Hardware: nested mode is the general test backend. DRM support is claimed
  only for explicitly tested hardware.

## Sprint 0 — Trustworthy baseline

### Implementation

- Replace the permissive verification script with fail-fast `make` targets:
  `check`, `check-e2e`, `check-apps`, `check-docs`, `check-package`,
  `check-all`, and `check-hardware`.
- Make Rust formatting, locked build/tests, and Clippy warnings hard failures.
- Keep every Scheme suite visible and fail on the first failed process.
- Check required tools before starting expensive tests.
- Remove tracked runtime logs and keep generated artifacts ignored.
- Record current behavior with characterization tests before architectural
  refactors.
- Replace the stale StumpWM gap analysis with a maintained capability table.

### Owner verification

Run from the repository root:

```sh
guix shell -m manifest.scm xorg-server xdotool imagemagick foot xterm \
  shellcheck -- make check-all
```

Expected: every required target passes and screenshots/logs are written only
under `/tmp/minde-e2e`. Then verify failure propagation:

```sh
PATH=/run/current-system/profile/bin make check-tools
```

Expected: a clear missing-Cargo error and a nonzero exit status. No TTY/DRM
test is required in this sprint.

## Sprint 1 — Licensing, provenance, and release contract

### Implementation

- Align Cargo, Guix, documentation, and package metadata on
  GPL-3.0-or-later.
- Add GPL and retained MIT license texts, copyright/provenance notices, and
  per-file SPDX identifiers where ownership differs.
- Add changelog, contributing/testing, security-reporting, and support-policy
  documents.
- Add strict `--help`/`--version`; reject unknown arguments and embed the build
  revision.
- Add automated license, provenance, version, and metadata checks.

### Owner verification

```sh
make check
cargo run -- --help
cargo run -- --version
cargo run -- --definitely-invalid
```

Expected: help and version exit successfully; the invalid option prints a
short error and exits nonzero. Inspect `LICENSES/` and `COPYING` and confirm
the Smithay-derived files retain an MIT notice.

## Sprint 2 — Reusable Scheme foundations

### Implementation

- Extract pure geometry/directional-neighbor, binary split-tree,
  serialization, hook, key notation/registry, prompt, and menu logic.
- Provide independently testable/installable `guile-minde-foundation` and
  `guile-minde-ui` monorepo packages.
- Keep compositor frames, groups, windows, protocols, and Rust adapters in the
  minde package.
- Stop exporting record internals merely to connect modules.

### Owner verification

```sh
make check-foundation
make check-ui
env -u DISPLAY -u WAYLAND_DISPLAY make check-foundation check-ui
guix build -f guix/foundation.scm
guix build -f guix/ui.scm
```

Expected: both packages build and test without Rust, a display server, or
global `wm-*` test stubs.

## Sprint 3 — Canonical API and atomic configuration

### Implementation

- Replace abbreviated StumpWM names with full kebab-case Scheme names.
  Predicates end in `?`, mutations in `!`, and event handlers begin with
  `handle-`.
- Curate public windows, frames, groups, layouts, input, commands, hooks, and
  status modules; keep FFI modules private.
- Introduce a command registry containing canonical name, procedure,
  arguments, category, summaries, full docs, and demo IDs.
- Load candidate configurations in isolation, validate them completely, and
  swap only after success.
- Add configuration validation without starting a display.

### Owner verification

```sh
make check-api
cargo run -- --check-config scheme/default-config.scm
scripts/mindectl check-config scheme/default-config.scm
```

While nested minde is running, introduce an invalid binding into a temporary
copy, request reload, and confirm that the error is reported while the previous
bindings continue to work. Restore the file and confirm a successful reload.

## Sprint 4 — Portable defaults and System integration

### Implementation

- Remove personal applications, de/bone, hostnames, scripts, wallpaper, eww,
  and autostart from repository defaults.
- Ship configurable terminal selection and a dependency-light default config.
- Adopt the prefix-only keymap:
  - `h/j/k/l`: focus frames; `w h/j/k/l`: move windows;
    `x h/j/k/l`: exchange windows.
  - `n` / `w p`: next/previous window; `w u n/p`: pull next/previous
    hidden window (`p` keeps the common pull-next action direct).
  - Digits select windows; `w` plus digits pulls them.
  - `r`: run prompt; `Space`: command palette; `:`: eval; `?`: help.
  - `w`, `f`, `g`, `m`, `s`: window, frame, group, layout/mode, and session
    submaps. Rare commands remain in the palette.
  - Avoid uppercase letter bindings; additional operations belong in submaps.
- Generate help from the command registry and reject collisions.
- Add a minde Guix Home service to `~/Projects/System` for the personal
  configuration while retaining StumpWM as a rollback session.
- Share package/service definitions across X1 and T450s and extend that
  repository's existing QA targets.

### Owner verification

Repository defaults:

```sh
make check-config check-keymaps
HOME="$(mktemp -d)" cargo run -- --winit
```

Expected: a clean-home nested session starts with no references to personal
paths or unavailable personal programs. In `~/Projects/System` run:

```sh
make qa-home-samuel
make qa-system-x1
make qa-system-t450s
```

After successful dry-runs, apply the Home configuration, log into minde,
and confirm the personal programs, Print prefix, de/bone layout, eww, and
autostart. Keep the StumpWM session selectable until after 1.0.

## Sprint 5 — Runtime safety and code cleanup

Status: implemented on 2026-07-13. Focused Scheme and Rust compile checks pass;
the owner verification below remains, including the full nested stress run.

### Implementation

- Serialize compositor mutations on the event-loop thread.
- Replace default threaded REPL mutation with main-thread IPC evaluation;
  retain the raw REPL only behind an explicit unsafe development option.
- Split oversized Rust state into output/backend, window, input, rendering,
  Xwayland, clipboard, IPC, and Guile-bridge responsibilities.
- Replace client/hotplug-reachable panics with contextual recovery.
- Audit popup, disconnect, hotplug, focus, clipboard, layer-shell, and reload
  lifecycles.
- Version and atomically write layouts, rules, and desktop state.

### Owner verification

```sh
make check check-e2e check-stress
scripts/mindectl eval '(current-group-name)'
```

During the stress test, repeatedly reload configuration and use the CLI while
clients map/unmap. Expected: no deadlock, panic, stale grab, or partial config.
Interrupt an atomic state write and confirm the previous file still loads.

## Sprint 6 — Diagnostics and stable IPC/status

Status: implemented on 2026-07-13. Scheme, Rust, Clippy, ShellCheck, release
metadata, and the nested e2e scenarios pass. Login-session log rotation,
hardware facts, and real-session privacy inspection remain owner checks.

### Implementation

- Add structured tracing fields and human/JSON log formats.
- Retain bounded current/previous session logs and Scheme stack traces.
- Add a redacted `mindectl report` containing versions, backend, kernel,
  outputs, GPU/DRM/seat summary, Xwayland state, config path, and recent errors.
- Add `query state --json`, atomic `status.json`, and streaming
  `subscribe --json`; version the schema.
- Include groups, focused group/window, urgency, outputs, and layout for bars.

### Owner verification

```sh
scripts/mindectl query state --json | jq .
timeout 5 scripts/mindectl subscribe --json
scripts/mindectl report --output /tmp/minde-report
```

Expected: valid versioned JSON and a self-contained report. Inspect the report
and confirm it excludes window titles, clipboard contents, command arguments,
unrelated environment variables, and unredacted home paths. Kill a nested
session and confirm both current and previous logs remain available.

## Sprint 7 — Layered test and application matrix

Status: implemented on 2026-07-13. Rust/Scheme properties, required core
applications, GTK 3, Qt 6, layer-shell clients, both nested keymaps and a
short soak passed internally. Optional browser/editor/toolkit shards, the
60-minute soak and owner visual inspection remain. Modern swaylock exposed a
missing `ext-session-lock-v1` protocol and is an explicit strict-matrix failure.

### Implementation

- Share Scheme fixtures and add property tests for tree invariants,
  directional focus, serialization, and output geometry.
- Add Rust unit tests for parsing, geometry, input translation, registries,
  status serialization, and rendering calculations.
- Build automated application scenarios for:
  - foot, basic Wayland clients, and wl-clipboard;
  - GTK 3/4 and Qt 5/6;
  - Electron/Chromium, Firefox, Emacs PGTK, and SDL2;
  - xterm and representative Xwayland clients;
  - eww, fuzzel and swaybg layer-shell behavior;
  - swaylock's modern session-lock requirement as an explicit unsupported
    result until `ext-session-lock-v1` exists.
- Keep Java, Wine, games, touch/tablet, IME, drag-and-drop, and multi-GPU as
  optional/manual coverage unless promoted into the tested matrix.
- Add soak loops for map/unmap, reload, group switching, clipboard, simulated
  hotplug, and Xwayland client churn. Keep an actual post-startup Xwayland
  server crash/restart as an explicit missing scenario.

### Owner verification

```sh
make check
make check-e2e
make check-apps-core
make check-apps-layer
make check-soak SOAK_MINUTES=60
```

Expected: each selected application produces a structured scenario result and
failures retain logs/screenshots. Run the heavyweight filters separately as
documented in `doc/application-testing.md`; do not combine their Guix closures.
Manually inspect popup/dialog placement, CSD, fullscreen, clipboard,
title/app-id changes, and layer exclusive zones. Swaylock remains a known
strict failure, not a successful lock test.

## Sprint 8 — Complete documentation and demonstrations

Status: implemented, pending owner verification on 2026-07-13. The README has been reduced to an overview;
focused tutorial, configuration, concepts, keymap, IPC/Eww, debugging,
architecture, security, hardware and support guides now exist. API/keymap/demo
manifests and an offline HTML manual are generated with drift/link checks. All
16 command scenarios produced validated three-second WebM clips, posters and
transcripts; 12 use real `C-t` input sequences and four unbound commands use
main-thread IPC. The generated API inventory documents all 274 live exports
from source docstrings, command metadata, or adjacent metadata for syntax and
value bindings. The unusually broad public boundary remains a pre-1.0 design
debt, but it is no longer hidden or undocumented. Sprint completion still
requires the owner to perform the offline-manual and sampled-video checks below.

### Implementation

- Replace the monolithic README with an overview plus tutorial, configuration,
  concepts, API, keymap, IPC/eww, debugging, architecture, security, hardware,
  and support documents.
- Make docstrings authoritative: signature, result, side effects, errors,
  example, and stability metadata for every public binding.
- Generate reference pages by enumerating the actual exports of all eight
  public modules and extracting procedure signatures and Guile docstrings.
  Merge command-registry metadata, including demo IDs, into those pages and
  reject undocumented or stale entries.
- Map every demonstrable public function or command to a version-controlled
  scenario through its demo ID. Classify non-visual APIs explicitly and give
  them executable text or log transcripts instead of meaningless videos.
- Run demo scenarios without manual interaction in a controlled nested
  compositor session. Fix the resolution, theme, applications, initial window
  state, timing, and input events in the scenario scripts.
- Capture short WebM clips and poster frames from the scenario runner, and
  generate a machine-readable manifest linking API names, demo IDs, scripts,
  transcripts, and assets.
- Treat generated videos as build/release artifacts rather than committed
  binaries. Use the same scenario runner locally and in CI: `check-docs`
  performs the fast documentation, coverage, and manifest-staleness checks;
  `demos` performs compositor capture and encoding; `check-demos` validates
  the resulting media and API coverage.

### Owner verification

```sh
make check-docs
make docs
make demos
make check-demos
```

Open the generated local documentation without network access. Follow the
fresh-install tutorial in a temporary home, run every executable snippet, and
sample each clip for correct input labels, visible result, and fallback poster.
Confirm that regenerating documentation and demos from a clean output
directory requires no manual recording or editing steps.

## Sprint 9 — Guix packaging and local release automation

Status: implemented, pending owner verification on 2026-07-13. The local and
archive-only package entry points, reproducible source/vendored archives,
version/schema checks, installed-content audit, sequential bounded release
runner, release notes, and checksums are in place. The full repository gate,
nested E2E, core application matrix, generated documentation, all scripted
videos, committed-archive reproducibility, offline Cargo metadata, local Guix
build, and archive-only Guix build pass. The personal System configuration
remains on the checkout until an actual RC artifact exists; SDDM installation
and login are Sprint 10 owner work.

### Implementation

- Separate local checkout packaging from a tag/archive-based public package.
- Generate a reproducible compressed vendored source archive for offline Guix
  builds without committing `vendor/`.
- Install binaries, Guile packages, helpers, documentation, licenses, portable
  config, and Wayland session entry.
- Validate version consistency across Cargo, Guix, CLI, docs, changelog, and
  schemas.
- Add `make release VERSION=...` to require a clean tree, run all non-hardware
  gates, build/test source and vendored archives offline, and generate release
  notes/checksums.

### Owner verification

```sh
make check-package
make release VERSION=1.0.0-rc1
```

In a clean temporary checkout with networking disabled, build the generated
Guix archive and inspect the output. Confirm SDDM finds the installed session,
all Scheme modules/docs are installed, and no build path references the
development checkout.

## Sprint 10 — Hardware validation, release candidate, and 1.0

### Implementation

- Record actual GPU/output/seat/scaling details and run the hardware checklist
  on X1 and T450s.
- Validate nested X11/Wayland, DRM login/logout, VT switching, suspend/resume,
  lid behavior, lock/unlock, session termination, available output/hotplug
  cases, input/media keys, clipboard, Xwayland, layer shell, fullscreen, and
  crash recovery.
- Run multi-hour soak sessions and fix all crash, data-loss, focus-trap, and
  security-sensitive failures.
- Freeze the public Scheme and persisted/status schemas at RC.
- Perform a clean installation from release artifacts before tagging 1.0.

### Owner verification

From a spare VT on each machine:

```sh
make check-hardware
```

Follow the generated checklist and retain its report. Rollback is always a VT
switch plus terminating minde; do not remove StumpWM or the preceding Guix
generation. Finally install the RC artifacts on a clean generation, log in
through SDDM, run `make check-soak SOAK_MINUTES=240`, and verify rollback to
the prior system generation before publishing 1.0.

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
