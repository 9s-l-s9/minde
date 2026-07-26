# Minde

Minde is a Wayland compositor with an embedded Guile policy layer. It
uses StumpWM's groups/heads/frames/window model, while Smithay owns
Wayland, Xwayland, rendering, input, outputs and backends.

The project is unreleased and breaking changes remain intentional before
1.0. The current support/evidence boundary is tracked in the
[capability matrix](capability-matrix.md).

## Start here

- [First session](tutorial.md) — a walkthrough from a cold checkout to a
  running nested compositor.
- [Concepts](concepts.md) — the groups/frames/window model.
- [Configuration](configuration.md) — the Guile init file and reload.
- [Architecture](architecture.md) — the Rust/Guile boundary.

## Reference

- [Scheme API reference](generated/api-reference.md) — live module exports
  and docstrings, generated from source.
- [Default keymap](generated/keybindings.md) — loaded key tables, generated.
- [Field manual](generated/manual.html) — a single-page index of the
  reference docs plus scripted command demonstrations (video capture is
  opt-in/local; see [demonstrations](demonstrations.md)).
- [IPC and Eww](ipc-eww.md) — the schema-v1 status contract external tools
  consume.

## Operating it

- [Debugging](debugging.md)
- [Diagnostics](diagnostics.md)
- [Testing](testing.md) and [application testing](application-testing.md)
- [Hardware validation](hardware-validation.md)
- [Security model](security.md)
- [Releasing](releasing.md)
- [Reusable packages](reusable-packages.md)

## Project status

- [Release roadmap](release-roadmap.md)
- [Support](support.md)
