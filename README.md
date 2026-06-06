# minde

A small Wayland compositor whose policy layer -- keybindings, spawning
programs, etc. -- lives in embedded Guile Scheme, StumpWM-style. The
compositor core is built on [Smithay](https://github.com/Smithay/Smithay).

## Credit

`src/state.rs`, `src/winit.rs`, `src/input.rs`, `src/handlers/`, and
`src/grabs/` are adapted from Smithay's `smallvil` example
(https://github.com/Smithay/Smithay/tree/master/smallvil), MIT licensed.
`src/input.rs` adds the Scheme key-dispatch hook described below; the rest
is close to unmodified aside from renaming `Smallvil` to `MindeState`.

Pinned Smithay revision: `3021f619e2ae4dab8bfb1e21f3f210923b9b6582`
(post-0.7.0, edition-2024 `master`; no crates.io release matches this
example's API yet, hence the git dependency in `Cargo.toml`).

## Build / run

This is a Guix project; the manifest pins Rust, Guile 3.0, and Smithay's
native dependencies (wayland, xkbcommon, libinput, seat, mesa, ...).

```sh
guix shell -m manifest.scm -- cargo build
guix shell -m manifest.scm -- sh -c 'LD_LIBRARY_PATH=$GUIX_ENVIRONMENT/lib cargo run'
```

`cargo run` opens a nested window (winit backend) under your host X11 or
Wayland session and runs minde inside it -- no DRM/seat access needed for
this MVP. Under X11, winit `dlopen`s libXi/libXcursor/libXrandr at runtime;
they're in the manifest, but dlopen only finds them via `LD_LIBRARY_PATH`
(hence the wrapper above -- unneeded if those libs are in your system
profile).

## Scheme layer

On startup the compositor calls `scm_init_guile()` on the main thread,
registers a few Rust-backed primitives, then loads `scheme/init.scm`
(override the path with `MINDE_INIT=/path/to/init.scm`).

Rust primitives available from Scheme:

- `(wm-spawn "cmd")` -- run `sh -c cmd` detached.
- `(wm-quit)` -- stop the compositor's event loop.
- `(wm-log "msg")` -- log via `tracing`.

`scheme/init.scm` defines `bind-key!` and a `wm-handle-key` dispatcher that
Rust calls on every key **press** (before forwarding to the focused
client) with `(mods-bitmask keysym keysym-name)`; returning `#t` consumes
the key. Modifier bits mirror common X11 masks: shift=1, ctrl=4, alt=8,
super=64.

Key bindings are StumpWM-style, behind the `C-t` prefix (see
`scheme/init.scm`): `C-t c` terminal, `C-t s`/`C-t S` vertical/horizontal
split, `C-t R` remove split, `C-t o` next frame, `C-t n` next window in
frame, `C-t k` close window; `super+q` quits. Frame layout policy lives in
`scheme/minde/frames.scm`; unit tests: `guile -L scheme
tests/frames-test.scm`.

Note when running nested inside StumpWM: StumpWM intercepts `C-t` itself --
forward it with StumpWM's `C-t t`, or change the prefix from the REPL,
e.g. `(set! %prefix-key "u")`.

`wm-handle-key` is looked up by name on every keypress rather than cached,
so redefining it (or the keybinding table) from a live REPL takes effect
immediately.

## Connecting to the REPL

`scheme/init.scm` starts a Guile REPL server on a Unix socket at
`$XDG_RUNTIME_DIR/minde-repl.sock` (or `/tmp/minde-repl.sock` if
`XDG_RUNTIME_DIR` is unset). Connect with a plain Guile REPL client:

```sh
guix shell guile -- guile
scheme@(guile-user)> ,use (system repl server)
scheme@(guile-user)> (system ("nc" "-U" "/run/user/1000/minde-repl.sock"))
```

or more simply, with `rlwrap`/`socat`:

```sh
socat readline unix-connect:$XDG_RUNTIME_DIR/minde-repl.sock
```

From Emacs with Geiser, use `geiser-connect-local` and point it at the
socket path. Note: the REPL runs in its own Guile-managed thread, so
mutating compositor state from it isn't synchronized with the main event
loop -- fine for redefining keybindings interactively, but be aware of
races if you touch other shared state.
