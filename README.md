# Minde

Minde is a Wayland compositor with an embedded Guile policy layer.  The
name is the Lojban word for "to command": every window-management
decision is a Scheme expression that can be inspected, rebound, or
evaluated live.  The window model (groups, heads, frames) follows
StumpWM; Smithay owns Wayland, Xwayland, rendering, input, and
backends.  Panels and trays are external -- Eww can consume the JSON
status interface.

This is a one-maintainer project.  Development version: `1.0.0-rc1`.
Breaking changes remain possible before 1.0.

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
