# Reusable Guile packages

These modules have no Rust or display-server dependency. They remain in the
monorepo but have separate Guix package definitions and verification targets.

## guile-minde-foundation

- `(minde foundation geometry)` — rectangle validation, edges, overlap,
  unions, and directional neighbor selection with caller-provided accessors.
- `(minde foundation tree)` — generic binary split trees, validation,
  mapping, folding, leaf traversal, and s-expression conversion.
- `(minde foundation serialization)` — exact single-datum string and file
  round trips. It rejects trailing data rather than silently ignoring it.
- `(minde foundation hooks)` — independent named hook registries with an
  injected error handler.
- `(minde foundation keys)` — Emacs-style modifier notation and a
  collision-detecting key registry.

The compositor currently reuses geometry for frame/head navigation,
serialization for layouts and placement rules, hooks through its adapter
module, and key notation in the key dispatcher.

## guile-minde-ui

- `(minde ui prompt)` — editable single-line prompt with completion,
  history, clipboard callbacks, and Emacs-like motion.
- `(minde ui menu)` — searchable, keyboard-driven menus with paging and
  direct digit selection.

Both UI modules require their side effects to be injected with
`configure-prompt-ui!` or `configure-menu-ui!`. Tests therefore use ordinary
lambdas rather than compositor globals.

## Verification

```sh
env -u DISPLAY -u WAYLAND_DISPLAY make check-foundation check-ui
guix build -f guix/foundation.scm
guix build -f guix/ui.scm
```
