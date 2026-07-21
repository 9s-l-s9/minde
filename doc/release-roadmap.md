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
- Sprint 10: release-candidate preparation implemented; T450s/X1 physical
  reports, four-hour soak, RC SDDM login, and rollback remain owner-pending.
- Sprint 11: implemented and internally verified on 2026-07-16. The nested
  entry point was restored and exercised live (build, IPC query, reload, and
  a mapped foot client), all documentation items landed with a generated,
  drift-gated `doc/testing.md`, and two out-of-scope defects found on the way
  were fixed: session-variable overrides ignored in compiled builds
  (declarative module), and a silently passing portable-defaults check when
  ripgrep was missing. Owner verification (clean-checkout README walkthrough
  and one interactive nested session) remains. Follow-up noted for a later
  sprint: make the IPC socket path overridable in `src/ipc.rs` instead of
  relying on runtime-directory isolation, and audit other Scheme modules
  exporting `set!`-able configuration for the same declarative-module
  pitfall.
- Sprint 12: implemented and internally verified on 2026-07-16. `scripts/ci`
  passes locally in both modes (default gates, and release artifacts with
  byte-reproducibility proven and a vendor-bootstrap path for fresh
  checkouts); the hosted workflow is YAML-thin by construction; the channel
  builds both Guile packages from committed files, with the compositor
  package deferred to the first published vendored archive; the packaging
  reference is generated and drift-gated. The first hosted Actions run on
  main passed on 2026-07-16 (run 29502921908) after one honest divergence was
  fixed: the pinned shell had no CA bundle, so cargo's TLS fetches failed on
  the runner while a host bundle masked the gap locally. On 2026-07-16 the
  `v1.0.0-rc1` tag run also passed end to end: the fresh-checkout
  `cargo vendor` bootstrap worked, both archives plus SHA256SUMS were
  published to the GitHub Release, and the published checksums are
  byte-identical to a local rebuild at the tag commit. The channel was
  verified through a real pull (clean clone via `guix time-machine`) after
  fixing a pull-only defect: compiled channel modules see `current-filename`
  as `#f`, so the repository root is now resolved through `%load-path`.
  Remaining: the repository is currently private, so the public-URL channel
  and release downloads activate only when it is made public; rerun the
  channel install from a second machine at that point.
- Sprint 13: implementation done (2026-07-16); owner hardware verification
  pending. `ext-image-copy-capture-v1` + `ext-image-capture-source-v1` are
  wired on both backends (`src/handlers/screencopy.rs`): shm capture
  everywhere, dmabuf constraints offered on udev for a future zero-copy
  screen-cast path; output capture only — no cursor sessions or toplevel
  sources are advertised. Captures are queued and satisfied after a real
  composited frame. Verified end to end nested: `grim` 1.5.0 produced a
  correct full-scene PNG, and a bounded gate (`tests/screencapture-e2e.sh`,
  part of `make check-e2e`) validates format, dimensions against the queried
  output mode, and non-flat content. Portal status is documented via the
  packaging-reference generator: `xdg-desktop-portal-wlr` in Guix speaks
  only `wlr-screencopy-unstable-v1`, which minde does not implement, so
  portal screen sharing is not yet available; the forward path is a portal
  backend speaking `ext-image-copy-capture-v1`. Remaining owner
  verification: udev/DRM capture on real hardware (screenshot + recording);
  the browser video-call check waits on the portal path. Owner verification
  progress (2026-07-16): DRM screenshot verified on X1 — after a system
  reconfigure and removing the shadowing home-profile install, `grim` captured
  a correct frame on real hardware. Recording and portal sharing remain
  blocked on `wlr-screencopy-unstable-v1` (wf-recorder 0.6.0 and
  xdg-desktop-portal-wlr in Guix speak only that protocol), so a follow-up
  sprint 13b implements it, reusing this sprint's render machinery — the
  approach sway/river use (compositor serves wlr-screencopy;
  xdg-desktop-portal-wlr bridges to PipeWire for browsers). Sprint 13b
  implementation done (2026-07-16): `zwlr_screencopy_manager_v1` v3 is
  hand-implemented in `src/handlers/wlr_screencopy.rs` (Smithay ships no
  server module) and reuses the ext machinery — both protocols queue into
  the shared pending-capture list satisfied after each composite. Region
  capture clips to the output; shm everywhere, dmabuf advertised on udev.
  The e2e gate now also proves a bounded `wf-recorder` recording nested.
  Remaining owner verification: on real hardware, record with `wf-recorder`
  and share a screen in one real browser call with `xdg-desktop-portal-wlr`
  + PipeWire running. Known follow-ups:
  pending frames for an unplugged output linger until the client drops the
  session; no periodic `ImageCopyCaptureState::cleanup()` call.
- Sprint 14: implementation complete (2026-07-17, commits 6b6430a..7490a30).
  All twelve items landed with gates green: data-control (wlr+ext) and
  primary selection incl. Xwayland; wlr-foreign-toplevel-management;
  wlr-output-management with the Scheme policy gate; pointer-constraints and
  relative-pointer; fractional-scale and viewporter with fractional-aware
  chrome; idle-notify and idle-inhibit; cursor-shape with Xcursor themes;
  text-input-v3/input-method-v2; virtual keyboard/pointer and
  keyboard-shortcuts-inhibit (lock-gated, focus-scoped); presentation-time,
  tearing-control (advisory only, no async-flip path in the vendored
  DrmCompositor) and linux-drm-syncobj (capability-guarded, udev-only);
  and the libinput configuration surface (wm-input-devices,
  wm-configure-input!). Remaining owner verification on real hardware:
  cliphist/clipboard manager, kanshi or wlr-randr layout change, a
  pointer-lock game or weston demo client, a fractional-scale display,
  swayidle auto-lock, fcitx5 round trip, wtype on the udev backend,
  presentation-time/syncobj behavior on the DRM backend, and per-device
  libinput rules taking effect.
- Sprint 15: not started. Agent interface — runtime API introspection, rich
  IPC error payloads, an event subscription stream, a one-command
  screenshot, and an optional thin MCP server, derived from five concrete
  LLM-agent workflows (see the sprint section). Post-protocol work: extends
  the IPC layer and tooling only, no frozen-module API changes.

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
- Automation: local Guix checks are authoritative for 1.0. Hosted CI is in
  scope (Sprint 12) but must call exactly the same noninteractive local entry
  point; no gate logic lives only in the hosted configuration.
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

Status: implemented, pending owner hardware verification from 2026-07-14. The
version is `1.0.0-rc1`; the public API, portable keymap, and configuration,
status, persistence, and diagnostic schema versions are frozen by
`release/contract.env`. `make check-hardware` now writes a retained report, and
the personal System/Home configuration can opt into a vendored RC archive
without removing its checkout or StumpWM rollback paths. T450s and X1 physical
sessions, four-hour soak runs, SDDM RC login, and generation rollback cannot be
claimed until the owner completes and retains both reports.

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

## Sprint 11 — Developer onboarding and debugging repairs

Status: implemented and internally verified 2026-07-16; owner verification
pending (clean-checkout README walkthrough plus one interactive nested
session). Source: the 2026-07-16 pre-release review. The documented first
command for a new contributor was broken, and the debugging story was only
half written down.

### Implementation

- Restore `scripts/run-nested` (referenced by README, `doc/tutorial.md`,
  `doc/debugging.md`, `PLAN.md`, and asserted executable by
  `tests/check-release-metadata.sh`), or update every reference to the real
  nested invocation. The release-metadata gate must pass again.
- Create `doc/testing.md` or remove/replace both dead README links to it.
  Prefer generating the verification-command index from `./check --help` and
  the Make targets rather than writing it by hand, so it cannot drift.
- Fix the stale `doc/support.md` claim that the session-lock protocol is
  missing and that modern swaylock is unsupported; `ext-session-lock-v1`
  landed with `(minde session)`.
- Document cold-build expectations: measured first-build time on reference
  hardware, the size of the vendored crate graph, and when (not) to add
  sccache/mold per the existing post-sprint policy.
- Document interactive Scheme debugging: a worked example of connecting to
  the `MINDE_UNSAFE_REPL=1` REPL (socket location, safety boundary) versus
  one-shot `mindectl eval`, in `doc/debugging.md`.
- Document Rust-side debugging: `RUST_BACKTRACE`, running the nested backend
  under gdb/lldb, constraints when the DRM backend owns the seat, and where
  the panic hook writes `crash.log`.
- Add short editor notes: rust-analyzer with the git-pinned/vendored Smithay
  dependency, and Geiser or guile-lsp for the Scheme tree.

### Owner verification

Follow README's "Quick nested session" section literally on a clean checkout
in a fresh `guix shell -m manifest.scm`; every command must work as printed.
Run `./check` and `make check-release-metadata`. Connect once to the unsafe
REPL following only the new documentation.

## Sprint 12 — Locally-runnable CI and scripted distribution

Status: implemented and internally verified 2026-07-16; owner verification
pending (first hosted Actions run, first tag-publish run, and a channel
install via `guix pull` from a clean machine). Decision: hosted CI is no
longer deferred, but the local
Guix checks remain authoritative — the hosted job must be a thin caller of the
same entry point a developer runs locally, never a second pipeline. All
packaging/distribution documentation is generated by scripts, not written by
hand.

### Implementation

- Add a single noninteractive CI entry point (for example `./check --ci` or
  `scripts/ci`) that composes existing gates: `./check`, doc-drift checks,
  release-metadata checks, and on tags the deterministic archive comparison.
  It must run identically inside `guix shell -m manifest.scm` on a developer
  machine and in the hosted runner.
- Add the hosted workflow (GitHub Actions) that installs Guix and executes
  exactly that entry point; no gate logic may live in workflow YAML.
- Publish release artifacts from the tag job: both deterministic archives and
  `SHA256SUMS` produced by the existing `make release-archives` path.
- Provide a Guix channel definition in-repo so Guix users can install without
  cloning; verify `guix pull`+install against it in the CI job.
- Generate downstream packaging material by script (a `make`/`scripts/`
  target emitting `doc/generated/packaging.md` or similar): runtime and build
  dependencies extracted from `guix.scm`/`manifest.scm`, installed file list,
  session-entry path, and archive URLs. No manually maintained packaging doc.
- Record the support boundary in generated output: first-party is pinned
  Guix; other distributions are community-packaged from the vendored archive.

### Owner verification

Run the CI entry point locally and confirm the hosted run executes the same
script with the same result. From a machine without the checkout, `guix pull`
the channel, install, and run `minde --version`. Confirm the generated
packaging document is reproduced byte-identically by rerunning its generator.

## Sprint 13 — Screen capture and desktop portal

Status: not started. This is the single remaining daily-driver dealbreaker:
without a capture protocol there are no screenshots, no screen recording, and
no video-call screen sharing. The vendored Smithay tree already contains the
protocol implementation; this is integration work, not protocol engineering.

### Implementation

- Implement `ext-image-copy-capture-v1` (and/or `wlr-screencopy-unstable-v1`
  for current `grim`/`slurp` compatibility) on both winit and udev backends.
- Verify `xdg-desktop-portal-wlr` (or -luminous equivalent) screen-cast via
  PipeWire against the chosen protocol set; document the required portal
  configuration in the generated packaging output from Sprint 12.
- Add a bounded nested e2e gate: capture a frame with `grim` (or the portal
  test client) and validate the image non-interactively.
- Update `doc/capability-matrix.md` from missing to supported/experimental in
  the same change, per the matrix's own update rule.

### Owner verification

On real hardware: take a screenshot, record a short screen capture, and share
a screen in one real browser video call. Retain the evidence per the hardware
checklist.

## Sprint 14 — Ecosystem protocol completion

Status: not started. Closes the remaining gaps a niri/hyprland/sway user hits
in the first days of real use. All protocols below have implementations in the
vendored Smithay revision; each item lands with handler wiring, a Scheme
surface where policy is involved, a bounded test, and a capability-matrix
update. Ordered by expected user pain.

### Implementation

- Clipboard ecosystem: `ext-data-control-v1`/`wlr-data-control` for clipboard
  managers, plus primary selection (middle-click paste) across Wayland and
  Xwayland.
- `wlr-foreign-toplevel-management` so external bars/switchers (including the
  Eww status consumers) can enumerate and activate windows.
- `wlr-output-management` so `wlr-randr`/`kanshi`/`wdisplays` can query and
  set output layout, mode, and scale; reconcile external changes with the
  Scheme head model.
- `pointer-constraints` and `relative-pointer` for games and pointer-lock
  clients.
- `wp-fractional-scale-v1` plus `wp-viewporter`, replacing integer-only
  scaling in the render paths.
- `ext-idle-notify-v1` and `idle-inhibit` so swayidle-style auto-lock works
  and fullscreen video/calls keep the screen awake; integrate with
  `(minde session)`.
- `cursor-shape-v1` and Xcursor theme/size loading, replacing the hardcoded
  fallback cursor bitmap.
- `text-input-v3`/`input-method-v2` for IME (fcitx5/ibus) and on-screen
  keyboards.
- `virtual-keyboard`/`virtual-pointer` for wtype/ydotool-style automation and
  accessibility tools.
- `keyboard-shortcuts-inhibit` for remote-desktop and VM clients.
- `wp-presentation-time`, `tearing-control`, and `linux-drm-syncobj`
  (explicit sync) in the udev backend.
- A libinput configuration surface in Scheme: per-device tap-to-click,
  natural scrolling, acceleration profile, and click method.

### Owner verification

Exercise each protocol with its canonical external client (cliphist,
wl-clipboard primary selection, kanshi, a pointer-lock game or test client,
a fractional-scale display, swayidle, fcitx5, wtype) on the nested backend
where possible and on real hardware where not, and retain the session report.

## Sprint 15 — Agent interface: REPL-driven control for humans and LLMs

Status: not started. The main-thread IPC eval socket, the state-as-data
surface (`dump-frames`/`restore-frames!`, `dump-desktop`, layout specs), and
the command catalog already make minde genuinely REPL-driven. This sprint
closes the remaining gap between "a scripting target" and "a surface an LLM
agent can drive in real time": runtime discoverability, self-correcting
errors, event push, and a screenshot/act toolset. No frozen-module API
changes; everything extends the IPC layer, the catalog, and tooling.

### Design: workflows first, architecture derived

The architecture is derived from five concrete workflows, ordered by how much
machinery each actually needs. The guiding rule: prefer the cheapest tier
that satisfies the workflow — a single LLM call beats an agent loop, and
generated persistent Scheme beats both.

- W1 — one-shot command ("move every terminal to group 2", "tile these two
  side by side"). One LLM call with the API catalog in context, emitting one
  s-expression for `mindectl eval`. Needs: a machine-readable catalog
  (names, signatures, docstrings, command schemas), data-only return values,
  and error payloads good enough to self-correct in one retry.
- W2 — visual arrange/verify ("make this look balanced", "which window is
  blank?"). An agent loop alternating screenshot → eval → screenshot. Needs:
  a one-command screenshot tool (the wlr-screencopy path from Sprint 13b
  already provides the protocol; this is packaging, not engineering) plus
  everything from W1.
- W3 — persistent automation ("whenever zoom opens, move it to group 3").
  The correct output is not a running agent: the LLM writes a Scheme hook
  once, validates it, and installs it into the user configuration. Runtime
  cost zero, no LLM in the loop after authoring. Needs: discoverable hook
  documentation in the catalog and a validate-then-reload path (both mostly
  exist: `mindectl check-config`, C-t R).
- W4 — reactive assistant (a daemon that watches events and occasionally
  consults an LLM: urgency triage, meeting-detected layout switches). Needs:
  an event subscription channel — the only workflow that cannot be built on
  today's poll-only observation (`status.json`).
- W5 — computer-use actuator. With `wm-send-key`, `wm-send-string`,
  `wm-click`, `wm-warp-pointer`, and virtual input already present, eval +
  screenshot makes minde a native computer-use backend for driving
  arbitrary applications — no extra compositor work, only documentation and
  the same screenshot tool as W2.

Derived decisions: the deliverable is a small, composable toolset —
describe, eval, subscribe, screenshot, send-input — not a built-in agent;
LLM/agent orchestration stays outside the compositor (same externalization
stance as the locker and idle daemon). An optional thin MCP server packages
those five tools for any MCP-speaking agent; it is a client of the public
sockets, never a new privilege domain. The eval socket remains full-power
`eval` under the user's own session (0600), documented as such: an agent
plugged into it has exactly the user's authority, which is the intended
trust model, not an oversight.

### Implementation

- Rich IPC errors: include the condition message and a bounded backtrace in
  the `(error ...)` payload so a model (or a human) can self-correct without
  guessing; keep the reply a readable datum.
- Runtime introspection: a `describe-api` entry point returning the full
  catalog — public procedures with signatures and docstrings, registered
  commands with schemas, hooks with their event payload shapes — as data
  over IPC, plus a generated machine-readable artifact (via
  `scripts/generate-docs`) suitable for dropping into an agent's context or
  tool definition.
- Writable-data guarantee: every public procedure reachable over IPC returns
  `write`-able data (no opaque records/procedures in replies), enforced by a
  test over the catalog.
- Event subscription: a read-only event socket (or `subscribe` verb)
  streaming the existing hook events as s-expression lines — window
  map/unmap/title, focus, group and head changes — with slow-consumer
  eviction so a stuck client cannot block the compositor.
- `mindectl screenshot [--output NAME] FILE`: wrap the existing capture
  path (grim over wlr-screencopy) into one predictable command for W2/W5.
- Optional `minde-mcp`: a thin stdio MCP server exposing describe / eval /
  subscribe / screenshot / send-input as tools, packaged like the other
  helper scripts; document the three integration tiers (one LLM call, agent
  loop, generated persistent Scheme) with one worked example each.
- Bounded e2e gate: scripted agent round trip nested — describe, eval a
  layout mutation, observe the matching event on the subscription socket,
  capture and validate a screenshot.

### Owner verification

Drive one real session end to end with an actual LLM agent (e.g. Claude Code
against `minde-mcp` or plain `mindectl`): a W1 one-shot command, a W2
screenshot-verify loop, and authoring a W3 hook that survives reload.
Retain the transcript as the evidence record.

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
