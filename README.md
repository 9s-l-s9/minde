# minde

*Lojban · [/ˈminde/](https://www.lojban.org/publications/reference_grammar/chapter3.html) · [to command](https://vlasisku.lojban.org/minde)*

A programmable Wayland compositor. Shape your desktop with Scheme, inspect its
state, and change its behavior while it runs.

![Rust + Guile](https://img.shields.io/badge/core-Rust_%2B_Guile-555555?style=flat-square)
![Release candidate](https://img.shields.io/badge/status-release_candidate-8a7040?style=flat-square)
[![GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-555555?style=flat-square)](COPYING)

Minde combines a Rust/Smithay compositor with an embedded Guile Scheme policy
layer. What you learn about the running system becomes code you can use every day.

## What you can do

- **Shape your workspace.** Combine manual frame trees, dynamic master/stack
  layouts, groups, multiple heads and floating windows. The window model follows
  StumpWM, with native Wayland clients and experimental Xwayland support.
- **Develop behavior live.** Compose Scheme procedures, hooks and keybindings;
  inspect state and evaluate expressions through the command prompt or IPC.
  Keep useful experiments in your configuration and reload it atomically.
- **Connect your tools.** Query versioned JSON state and subscribe to events.
  External panels such as Eww can use the same status interface; Scheme commands
  can combine window operations with your own logic.

## How it fits together

```text
Rust / Smithay                 Guile / Scheme
protocols, rendering, input  <-> groups, frames, layouts
backends and event loop         commands, hooks, configuration
```

Rust owns the display machinery and exposes small primitives. Scheme owns the
window-management decisions. Supported live evaluations run on the compositor
thread, so you can change policy through the same execution path used by keys
and hooks. Configuration reload validates the candidate before replacing active
tables. See the [architecture](doc/architecture.md) and [Scheme API](doc/api.md).

Minde is for developers who enjoy programming their working environment and
want that investment to carry into their daily desktop. It is a one-maintainer
project. Development version: `1.0.0-rc1`. Breaking changes remain possible
before 1.0.
The [capability matrix](doc/capability-matrix.md) distinguishes supported features
from experimental protocols and hardware paths.

## Trying it

The nested backend runs in a window and does not touch DRM or the
display manager:

    guix shell -m manifest.scm
    scripts/run-nested

The prefix key is C-t: `C-t Return` opens a terminal, `C-t ?` shows
contextual help.  See doc/tutorial.md for a first session and
doc/generated/keybindings.md for the complete key map.

## Configuration

The entry point is scheme/init.scm; scheme/default-config.scm is a
versioned data expression.  Configuration is validated and reloaded
atomically:

    scripts/mindectl check-config scheme/default-config.scm
    scripts/mindectl eval '(reload-configuration!)'
    scripts/mindectl query state --json

The IPC socket belongs to the session user and runs requests on the
compositor thread.  doc/ipc-eww.md describes the status and event
interfaces.

## Documentation

Start with doc/index.md.  Notable entries: doc/concepts.md (window
model), doc/api.md (Scheme API), doc/architecture.md,
doc/debugging.md, doc/security.md.  Generated references
(doc/generated/) are committed so they stay readable without build
tools; `make docs` regenerates them and `make check-docs` rejects
drift.

## Development

Enter `guix shell -m manifest.scm` once per session, then run

    ./check

before committing.  It runs the fast Rust, Scheme, API, configuration,
keymap, and documentation gates; `./check --help` lists the optional
deeper suites.  Anything touching DRM, libinput, or VT switching needs
real hardware; see doc/hardware-validation.md.

Two Scheme libraries are usable outside the compositor:
guile-minde-foundation (geometry, trees, hooks, key notation) and
guile-minde-ui (prompt and menu state machines).  See
doc/reusable-packages.md.

## License

Project code is GPL-3.0-or-later.  Parts of src/ are adapted from
Smithay's MIT-licensed smallvil example; the pinned revision and exact
provenance are recorded in NOTICE.  See COPYING and LICENSES/.

Please read SUPPORT.md, SECURITY.md, and CONTRIBUTING.md before
reporting issues or sending patches.
