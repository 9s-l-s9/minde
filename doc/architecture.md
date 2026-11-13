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
debt must be curated before API stability is claimed.

## Reusable packages

`guile-minde-foundation` contains pure geometry, tree, serialization, hook,
and key-notation code. `guile-minde-ui` contains prompt/menu state machines
with injected display operations. They build and test independently; see
[`reusable-packages.md`](reusable-packages.md).

## State and persistence

Native xdg toplevel roles remain unmanaged until their first buffer-bearing
commit. A null-buffer commit unmaps the window while keeping its role alive:
the layout entry and foreign-toplevel handles are removed, focus moves on,
and associated grabs and popups end. A subsequent map receives a fresh window
ID, so stale selectors cannot address it. Initial title/app-id and fullscreen
requests are applied when the first buffer maps.

Application fullscreen requests address the requesting window through
`handle-window-fullscreen!`. Each group's fullscreen state tracks its owner
separately from keyboard focus; a hidden group applies the requested geometry
when activated. Taskbar fullscreen combines validated activation with the same
ownership operation. Invalid or unselectable IDs do not alter another window.
Xdg requests receive a configure even when deferred or already satisfied.

Interactive resize completion follows the final configure's acknowledgement
on a surface commit. Older commits keep their opposite edges anchored until
that acknowledgement arrives. Policy receives the requested final rectangle
at button release, then the actual committed rectangle if the client chose
a different size. Cancelling a resize clears its positioning state, sends a
configure without the resizing flag, and retains the committed rectangle.

Native and X11 windows share unmap cleanup, including active move/resize
grabs. On Xwayland connection loss, managed X11 windows and unmanaged menus
are removed, its selections are cleared, and runtime status becomes
`failed` with no X display. Native clients and their selections survive.
Xwayland is not automatically restarted. Children launched by `wm-spawn`
receive only the current embedded X display; while it is unavailable,
their `DISPLAY` is removed without changing the compositor's environment.

Runtime state is authoritative in the compositor thread. Status publication
uses versioned JSON and atomic files. Layouts and placement rules use versioned
Scheme datums and atomic replacement. Configuration reload validates a complete
candidate before swapping active tables.

## Command application and state ownership

`wm-*` primitives apply their command to the compositor state immediately
when Scheme runs on the main thread (every hook, key, timer and IPC
evaluation), and return whether the command found its target. Only the
optional REPL thread, and Scheme code that Rust calls back into from inside
a command, go through the calloop channel. `sync-frames-now!` therefore
computes its whole placement plan first, sends it as one `wm-place-windows`
batch, and only then updates focus bookkeeping: a policy error surfaces
before anything reaches the compositor.

Each fact has one owner; the other side keeps at most a cache:

| Fact | Owner | Mirror | Reconciled by |
|---|---|---|---|
| Window title, app-id | Rust (`wm-window-title`) | `%window-titles` cache, rename overrides | `handle-window-title-change!` |
| Window geometry | Rust | placed-rect cache | `wm-place-window` result |
| Outputs/heads | Rust (`wm-outputs`) | `%heads` (per-head or span) | `handle-heads-change!` |
| Floating set | Scheme (`%floating`) | `floating_ids` | `wm-set-floating` |
| Frames, groups, focus policy | Scheme | none | n/a |

`(mirror-drift)` compares every mirror with its owner and returns the
discrepancies; the diagnostic report writes it to `drift.txt` and the e2e
suite requires it to be empty.

## Callback contract

Rust invokes the policy layer through the names in the `Hook` table
(`src/guile/mod.rs`). `(minde compositor callbacks)` records each name's
argument count, `define-compositor-callback!` installs a replacement after
checking it, `(compositor-callbacks)` lists every name with its status, and
`check-compositor-callbacks!` runs at startup and on each configuration
reload, logging unbound or mis-arity definitions. `tests/callbacks-test.scm`
keeps the registry and the Rust table identical.

## Timing probes

`src/timing.rs` records the duration of every applied command, key dispatch
and rendered frame in lock-free histograms. `(wm-timing-stats)` returns
them; the diagnostic report writes `timing.txt`. `make bench-scheme` times
the frame-sync path against stubbed primitives.
