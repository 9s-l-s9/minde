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
those programs are installed. **Ctrl+Alt+BackSpace** is the native emergency
exit: it immediately ends the entire session without asking applications to
save. It works through broken Scheme, shortcut inhibitors and session locks.
Use `C-t s q` for a normal, confirmed logout. The declarative
`emergency-exit` setting can rebind or disable the emergency chord; Super+Q
has no repository-default action.
These direct bindings are included in the generated table instead of being
maintained in a second list.

## Help and discoverability

Binding descriptions are stored beside live binding objects. Nested maps are
registered as relationships even when entering one first shows a window list
or frame-number overlay. This lets contextual help and generated documentation
walk the same map without invoking arbitrary actions.

Personal bindings and the historical map can use `fselect!` (select a frame)
or `expose-mode!` (show windows in a temporary grid and select one). Their
labels use digits followed by letters, with equal-width multi-key labels
above 36 choices. An unbound key, Escape, Return, or `C-g` cancels selection
and clears its overlays; expose also restores the saved layout. Group/head
changes and session locking cancel these modes automatically.

The historical full keymap remains available through
`MINDE_FULL_KEYMAP=1` for established configurations. It is not the release
default and is not a compatibility promise before 1.0.
