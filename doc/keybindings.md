# Keybinding guide

The authoritative repository-default table is generated from the actual
loaded keymaps in [`generated/keybindings.md`](generated/keybindings.md). Run
`make docs` after changing a binding; `make check-docs` rejects stale output.
All default prefix and global bindings are data in
[`scheme/default-config.scm`](../scheme/default-config.scm), validated and
reloaded together. See [configuration](configuration.md) for overrides,
typed command arguments, nested maps and unbinding.

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

The default global map is empty. Add personal brightness, audio and launcher
commands in your configuration. **Ctrl+Alt+BackSpace** is the native emergency
exit: it immediately ends the entire session without asking applications to
save. It works through broken Scheme, shortcut inhibitors and session locks.
Use `C-t s q` for a normal, confirmed logout. The declarative
`emergency-exit` setting can rebind or disable the emergency chord; Super+Q
has no repository-default action.
The native chord is included in the generated table. `C-t Return` and `C-t r`
open the run prompt with PATH completion; no terminal program is assumed.

Prompts and menus accept both Return and keypad Enter. They consume text keys
and their matching releases while forwarding modifier keys such as Shift and
Control, so applications keep their modifier state when the prompt closes.
Shortcut and prefix releases also remain consumed if focus changes, an
application changes shortcut inhibition, or the session locks while a key is
held. Keys already forwarded to applications keep their matching releases.

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

The historical full keymap lives in `scheme/keymaps/stumpwm.scm` and loads only
when selected through `MINDE_FULL_KEYMAP=1` or explicitly by a personal init.
It is not the release default or a compatibility promise before 1.0.
