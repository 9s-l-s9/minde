# Architecture

Minde is a Smithay compositor with policy implemented in embedded Guile.
Rust owns protocol objects, rendering, input devices, backends and event-loop
integration; Scheme owns groups, frames, commands, layouts, hooks, prompts and
configuration.

## Event flow

```text
Wayland/X11/input event
  -> Smithay handler on the calloop thread
  -> registered Guile handler or wm-handle-key
  -> Scheme policy mutation
  -> Rust placement/focus/render primitive
  -> status publication and hooks
```

All supported mutation paths return to the event-loop thread. The IPC socket is
a calloop source; it does not mutate Guile from a worker. The optional unsafe
REPL is clearly separated because it cannot provide that guarantee.

## Rust responsibilities

- winit nested and DRM/udev/libinput backends;
- xdg-shell, Xwayland, layer-shell, clipboard and output protocol handling;
- rendering, damage, focus, pointer/keyboard grabs and output confinement;
- IPC framing, structured tracing and crash evidence;
- registration of small `wm-*` Guile primitives.

## Scheme responsibilities

The advertised public modules are `(minde windows)`, `frames`, `groups`,
`layouts`, `input`, `commands`, `hooks`, and `status`. The live generated
inventory is [`generated/api-reference.md`](generated/api-reference.md).

Modules under `(minde compositor ...)` are intended as implementation
details. The current boundary still exposes internal frame/model helpers; that
debt is recorded in `UNEXPECTED.md` and must be curated before API stability is
claimed.

## Reusable packages

`guile-minde-foundation` contains pure geometry, tree, serialization, hook,
and key-notation code. `guile-minde-ui` contains prompt/menu state machines
with injected display operations. They build and test independently; see
[`reusable-packages.md`](reusable-packages.md).

## State and persistence

Runtime state is authoritative in the compositor thread. Status publication
uses versioned JSON and atomic files. Layouts and placement rules use versioned
Scheme datums and atomic replacement. Configuration reload validates a complete
candidate before swapping active tables.
