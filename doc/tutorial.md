# First nested Minde session

This tutorial starts Minde in a window. It does not touch the login
session, DRM devices, personal configuration, wallpaper, Eww, or autostart.

## Build and start

```sh
git clone https://github.com/9s-l-s9/minde
cd minde

guix shell -m manifest.scm -- cargo build --locked
guix shell -m manifest.scm -- sh -c '
  export LD_LIBRARY_PATH="$GUIX_ENVIRONMENT/lib"
  export MINDE_INIT="$PWD/scheme/init.scm"
  export MINDE_SCHEME_DIR="$PWD/scheme"
  export MINDE_CONFIG="$PWD/scheme/default-config.scm"
  cargo run --locked -- --winit
'
```

The nested window uses the dependency-light repository configuration. Its
prefix is `C-t`; a personal Guix Home configuration may replace that with
`Print` without changing the repository default.

## Basic interaction

Inside the nested compositor:

1. Press `C-t Return` to open foot, falling back to xterm.
2. Press `C-t ?` to show contextual help.
3. Open a second terminal and use `C-t n` / `C-t N` to cycle windows.
4. Use `C-t f h` or `C-t f v` to split the current frame.
5. Press `C-t f`; numbered frame overlays appear. Select one with `0`–`9`.
6. Press `C-t w`; numbered windows appear. `w 0`–`w 9` pull one into the
   current frame.
7. Exit with `C-t s q`, or use the emergency direct binding `super+q`.

The complete table is generated from the loaded keymaps in
[`generated/keybindings.md`](generated/keybindings.md).

## Inspect state

While the nested compositor is running, use another terminal:

```sh
scripts/mindectl query state --json | jq .
scripts/mindectl eval '(current-group-name)'
timeout 5 scripts/mindectl subscribe --json
```

These commands use the serialized main-thread control socket, not the unsafe
development REPL.

## Make a small configuration change

Copy the declarative default and change its prefix or add a registered
zero-argument command:

```sh
cp scheme/default-config.scm /tmp/minde-config.scm
scripts/mindectl check-config /tmp/minde-config.scm
```

Restart with `MINDE_CONFIG=/tmp/minde-config.scm`, or evaluate
`(reload-configuration!)` after editing. Invalid data is rejected before the
active binding table changes. Continue with
[`configuration.md`](configuration.md) before adding imperative policy.

## Next checks

Run `make check` for code and configuration tests. Nested graphical tests and
the bounded application matrix are documented in
[`application-testing.md`](application-testing.md). A real DRM/login-session
test is deliberately separate; follow
[`hardware-validation.md`](hardware-validation.md) from a spare VT.
