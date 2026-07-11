# Minde

Minde is a Wayland compositor with an embedded Guile policy layer. It uses
StumpWM's groups/heads/frames/window model, while Smithay owns Wayland, Xwayland,
rendering, input, outputs and backends. The modeline, panel and tray are
deliberately external; Eww consumes the structured status interface.

The project is unreleased and breaking changes remain intentional before 1.0.
Development version: `1.0.0-rc1`.
The current support/evidence boundary is tracked in
[`doc/capability-matrix.md`](doc/capability-matrix.md).

## Highlights

- Manual binary frame trees and dynamic master/stack groups.
- Group-wide window cycling, numbering, marks, placement rules and layouts.
- Floating, fullscreen, urgency, key synthesis, prompts, menus and hooks.
- Multiple outputs with per-head or spanning trees and hotplug adoption.
- Native Wayland, Xwayland, clipboard and layer-shell support.
- Atomic declarative reload plus serialized main-thread Scheme IPC.
- Schema-v1 JSON status for Eww and other external tools.
- Structured logs, retained crash evidence and redacted report bundles.
- Reusable Guile foundation and UI packages.
- Generated API/keymap references and scripted WebM demonstrations.

Session lock, suspend and logout commands (`Print s l` / `s z` / `s q` on the
portable map) drive `ext-session-lock-v1` through a configurable external
locker (default `swaylock -f`); see [Session management](#session-management)
below. Layer shell must not be presented as a secure replacement for it.

## Quick nested session

The nested backend runs in a window and does not touch DRM, the display
manager, or personal startup policy:

```sh
guix shell -m manifest.scm
scripts/run-nested
```

Keep that shell and compositor running while editing. After a Scheme change,
run `./check` and then
`scripts/mindectl eval '(reload-configuration!)'` from another terminal;
Cargo does not need to rebuild.

The repository default prefix is `C-t`: use `C-t Return` for a terminal and
`C-t ?` for contextual help. A personal Guix Home configuration may replace
the prefix with `Print`; generated repository documentation describes the
repository default rather than silently mixing the two.

Continue with [`doc/tutorial.md`](doc/tutorial.md). The complete accepted map
is generated from the live key tables in
[`doc/generated/keybindings.md`](doc/generated/keybindings.md).

## Documentation

| Topic | Document |
|---|---|
| First session | [`doc/tutorial.md`](doc/tutorial.md) |
| Configuration and startup | [`doc/configuration.md`](doc/configuration.md) |
| Groups, heads, frames, windows | [`doc/concepts.md`](doc/concepts.md) |
| Keybinding conventions | [`doc/keybindings.md`](doc/keybindings.md) |
| Scheme API overview | [`doc/api.md`](doc/api.md) |
| Generated API inventory | [`doc/generated/api-reference.md`](doc/generated/api-reference.md) |
| IPC, status, and Eww | [`doc/ipc-eww.md`](doc/ipc-eww.md) |
| Debugging | [`doc/debugging.md`](doc/debugging.md) |
| Complete verification command reference | [`doc/testing.md`](doc/testing.md) |
| Architecture | [`doc/architecture.md`](doc/architecture.md) |
| Security model | [`doc/security.md`](doc/security.md) |
| Hardware/login validation | [`doc/hardware-validation.md`](doc/hardware-validation.md) |
| Support and compatibility | [`doc/support.md`](doc/support.md) |
| Application matrix | [`doc/application-testing.md`](doc/application-testing.md) |
| Scripted demonstrations | [`doc/demonstrations.md`](doc/demonstrations.md) |
| Packaging and releases | [`doc/releasing.md`](doc/releasing.md) |
| Offline generated manual | [`doc/generated/manual.html`](doc/generated/manual.html) |

`make docs` regenerates API, keymap, manifest and HTML outputs. Generated files
are committed so they remain readable without build tools; `make check-docs`
rejects drift. Videos, posters and transcripts are reproducible build artifacts
under `build/demos`, not committed binaries:

```sh
make demos check-demos
```

The sixteen clips are captured sequentially with one compositor and one
two-thread encoder.

## Configuration and control

The portable entry point is `scheme/init.scm`; `scheme/default-config.scm` is a
versioned data expression. Validate and reload without partially replacing the
active key table:

```sh
scripts/mindectl check-config scheme/default-config.scm
scripts/mindectl eval '(reload-configuration!)'
scripts/mindectl query state --json | jq .
timeout 5 scripts/mindectl subscribe --json
```

The IPC socket belongs to the session user and runs requests on the compositor
event-loop thread. `MINDE_UNSAFE_REPL=1` is an explicitly unsafe development
escape hatch, not a normal automation interface.

Personal launchers, keyboard policy, wallpaper, Eww, brightness initialization
and autostart belong in the personal configuration. Editing a Guix Home service
does not affect the current session until Home is reconfigured and Minde is
restarted.

## Session management

`(minde session)` provides three interactive commands, also reachable from
the colon prompt (`Print colon`):

| Command | Portable default | Full keymap (`MINDE_FULL_KEYMAP=1`) | What it does |
|---|---|---|---|
| `logout!` | `s q` | `Q` | Prompts `log out (ends the session)? (yes/n)`; only `y`/`yes` calls `wm-quit`, ending the session. |
| `lock-screen!` | `s l` | `L` | Spawns `%lock-command` (default `"swaylock -f"`). |
| `suspend!` | `s z` | `M-L` | Locks first, waits for confirmation, then spawns `%suspend-command` (default `"loginctl suspend"`, elogind-compatible). |

`suspend!` does not suspend the instant it spawns the locker: spawning races
the locker's own startup, and suspending mid-race can wake the machine
unlocked. It waits for the compositor's `wm-on-session-lock` event (fired once
the session-lock surface is actually up) before suspending, and gives up
without suspending if that does not happen within `%lock-timeout-ms`
(default 5000) -- failing closed, since suspending anyway on an unconfirmed
lock would be a worse bug than a suspend that has to be retried by hand. Set
`%lock-on-suspend?` to `#f` to suspend immediately without locking. Override
any of `%lock-command`, `%suspend-command`, `%lock-on-suspend?`, or
`%lock-timeout-ms` after `(use-modules (minde session))` in a personal
configuration.

## Development and verification

Enter `guix shell -m manifest.scm` once per development session. The manifest
contains the compiler, native libraries, bounded core application tests,
documentation tools, and video encoder. Browsers and large
toolkit matrices remain opt-in so the environment stays reasonable.

Normal development needs one command: `./check`. It always runs the same fast
Rust, Scheme, API, configuration, keymap, static, and documentation gates.
Rust uses `cargo check` rather than final linking, so warm runs remain fast:

```sh
./check
```

Run the same command before committing. Focused Scheme tests and the bounded
integration suite are optional escalation paths documented by
`./check --help`; they are not additional setup steps. The complete matrix is
indexed in [`doc/testing.md`](doc/testing.md). Implementation-level `make
check-*` targets exist for CI and diagnosing a failed gate.

Packaging, deterministic videos, and archive checks are deliberately outside
the edit loop:

```sh
./check --release
make check-hardware
```

Anything involving DRM, libinput, VT switching or real output hotplug requires
the owner checklist in [`doc/hardware-validation.md`](doc/hardware-validation.md).

## Reusable Scheme packages

`guile-minde-foundation` provides geometry, binary trees, serialization,
hooks and key notation. `guile-minde-ui` provides prompt/menu state machines
with injected display operations. Both test and build without a compositor;
see [`doc/reusable-packages.md`](doc/reusable-packages.md).

## Provenance, license, and support

`src/state.rs`, `src/winit.rs`, `src/input.rs`, `src/handlers/`, and `src/grabs/`
are adapted from Smithay's MIT-licensed `smallvil` example. The pinned Smithay
revision is `3021f619e2ae4dab8bfb1e21f3f210923b9b6582`; exact provenance is in
[`NOTICE`](NOTICE).

Project-owned code is GPL-3.0-or-later. Retained Smithay-derived code remains
available under MIT. See [`COPYING`](COPYING) and [`LICENSES`](LICENSES).

This is a one-maintainer pre-1.0 project. Read [`SUPPORT.md`](SUPPORT.md),
[`SECURITY.md`](SECURITY.md), and [`CONTRIBUTING.md`](CONTRIBUTING.md) before
reporting or contributing.
