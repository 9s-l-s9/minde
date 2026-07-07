# Keybinding guide

The authoritative repository-default table is generated from the actual
loaded keymaps in [`generated/keybindings.md`](generated/keybindings.md). Run
`make docs` after changing a binding; `make check-docs` rejects stale output.

## Modal convention

The portable map follows a Helix/Meow-like direct modal vocabulary:

- `h/j/k/l`: directional frame focus;
- `w h/j/k/l`: move the current window between frames;
- `n`: next group window; `w p`: previous group window;
- `p`: pull the next hidden window; `w u n/p`: pull the next/previous
  hidden window;
- `w`, `f`, `g`, `m`, `s`: window, frame, group, layout/mode, and session maps;
- digits select numbered windows directly;
- `w 0`–`w 9` pull numbered windows;
- `f 0`–`f 9` select numbered frames;
- `x h/j/k/l` exchange windows directionally;
- `?` displays help for the currently armed map.

The portable map deliberately has no uppercase letter bindings. Additional
actions use lowercase submaps instead of requiring Shift.

The repository prefix is `C-t`. The maintainer's Guix Home layer deliberately
uses `Print`; generated repository documentation does not pretend those are
the same configuration.

## Direct keys

Brightness and audio keys spawn `brightnessctl` and `wpctl` commands when
those programs are installed. `super+q` is an emergency compositor exit.
These direct bindings are included in the generated table instead of being
maintained in a second list.

## Help and discoverability

Binding descriptions are stored beside live binding objects. Nested maps are
registered as relationships even when entering one first shows a window list
or frame-number overlay. This lets contextual help and generated documentation
walk the same map without invoking arbitrary actions.

The historical full keymap remains available through
`MINDE_FULL_KEYMAP=1` for established configurations. It is not the release
default and is not a compatibility promise before 1.0.
