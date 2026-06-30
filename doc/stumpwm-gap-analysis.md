# StumpWM → minde gap analysis (toward 1.0.0)

Source of truth: StumpWM 24.11 (`defcommand` inventory from the store
sources, the info manual, and `primitives.lisp` hook list), compared
against minde at commit `a34601c` (2026-07-11).

Legend: ✅ have (parity or deliberate equivalent) · 🟡 partial ·
❌ missing · 🚫 excluded by decision.

## Already covered

| StumpWM | minde |
|---|---|
| prefix key, `set-prefix-key`, `bind`, nested keymaps | ✅ `set-prefix-key!`, `bind-prefix-key!`, `make-keymap` |
| literal forward (prefix twice) | ✅ (repurposed to frame-cycle when bound; unbind restores) |
| `hsplit` / `vsplit` / `remove-split` | ✅ |
| `fnext`, `next`, `pull-hidden-next`, `other-in-frame` | ✅ f/n/p/o with StumpWM semantics |
| `windowlist`, `select-window` | ✅ native prompt with completion |
| `delete-window` | ✅ d/k (polite close) |
| `echo`, message window, `*timeout-wait*` | ✅ wm-message + timeout |
| `read-one-line` input bar (editing keys, TAB completion, history) | ✅ (minde input) |
| groups: `gnew`, `gnext`, `gprev`, `gmove` | ✅ |
| `resize-direction` (iresize) + `balance-frames` | ✅ Print s mode |
| `dump-group-to-file` / `restore-from-file` | ✅ (minde layouts) — group-level only |
| `define-frame-preference` / `place-existing-windows` on map | ✅ sprint 7: `place-existing-windows!` (Print P p); lock/raise flags still 🚫 (follow? covers the common case). |
| `run-shell-command` | ✅ wm-spawn + run prompt |
| `loadrc` / `reload` | ✅ Print R (init.scm only; modules need re-login) |
| REPL (`stumpish`/slime) | ✅ REPL socket + minde-cmd |
| mode-line + system tray | 🚫 excluded: eww + status-line file instead |

## Missing — pure Scheme (cheap, high value; suggested 1.0 core)

| Feature | StumpWM commands | Notes |
|---|---|---|
| **Directional frame focus** | `move-focus` (left/right/up/down) | Frames know their rects; pick nearest frame in direction. StumpWM binds arrows on the prefix map. |
| **Directional window move/swap** | `move-window`, `exchange-direction` | Same geometry math, move current window to neighbor frame. |
| **Reverse cycling** | `prev`, `prev-in-frame`, `pull-hidden-previous`, `fprev` | We only cycle forward everywhere. |
| **Last-window / last-group toggle** | `other-window` (StumpWM's default C-t C-t), `gother`, `fother` | Needs a "previous focus" slot per group/screen. |
| **Window numbers** | `select-window-by-number`, `pull-window-by-number`, `renumber`, `repack-window-numbers`, `echo-windows` showing `0*Term 1-Editor` | StumpWM's bread and butter: Print 0–9 jumps. We only have internal ids. |
| **`only`** | collapse tree to one frame keeping current window | trivial with layouts (`apply-layout-spec! 'leaf`) |
| **`fclear`, `curframe`** | empty a frame; flash current frame indicator | |
| **Equal splits** | `hsplit-equally`, `vsplit-equally` | split into N columns/rows |
| **Group management** | `grename`, `gselect`/`grouplist` (prompt), `gkill`, `gmerge`, `gnewbg`, `gmove-and-follow` | We can only create/cycle/move-without-follow. |
| **Eval prompt** | `colon`, `eval-line` | read-one-line → eval in (guile-user); we have all pieces. |
| **Help system** | `commands`, `describe-key`, `where-is`, `which-key-mode` | which-key: echo the armed keymap's bindings after a short delay — big usability win, easy (we own the keymaps + sticky messages). |
| **User hooks** | `*focus-window-hook*`, `*new-window-hook*`, `*focus-group-hook*`, ... (36 hooks) | Scheme hook lists (`add-hook!`-style) run from the existing event paths; makes user config extensible without patching modules. |
| **`lastmsg` / `copy-last-message`** | re-show / copy last message | keep a message ring in Scheme. |
| **Marks** | `mark`, `gmove-marked`, `pull-marked`, `clear-window-marks` | batch window ops. |
| **`quit-confirm`** | y/n prompt before quit | one read-one-line. |
| **`title` (rename window), `echo-date`** | ✅ sprint 7: `rename-window!` (Print P n); echo-date = `(echo (strftime ...))` one-liner. |
| **`command-mode`** | prefix-less modal bindings | ✅ post-sprint-7: `command-mode!` (Print z); every key acts as a prefix key until Return/C-g/Escape. |

## Missing — needs Rust support (small, contained)

| Feature | StumpWM | Rust work |
|---|---|---|
| **Fullscreen** | `fullscreen` | ✅ sprint 3: `fullscreen!` (Print M-f), layout frozen while active. |
| **Force kill** | `kill-window` | ✅ sprint 3: `kill-current-window!` (Print K) drops the client connection. |
| **Pointer control** | `banish`, `ratwarp`, `ratclick` | ✅ sprint 3: `banish!` (Print B) + `ratwarp!`; sprint 7: `ratclick!`. |
| **Urgency** | `next-urgent`, `*urgent-window-hook*` | ✅ sprint 3: xdg-activation → 'urgent-window hook + `next-urgent!` (Print C-u). |
| **Send string to window** | `window-send-string` | ✅ sprint 7: `(window-send-string text)` -- synthesized key events; chars must exist at shift level 0/1 of the active layout. |
| **Always on top** | `toggle-always-on-top` | ✅ sprint 7: `toggle-always-on-top!` (Print P t), re-raised above floats every sync. |
| **Frame indicator** | `curframe`, `*suppress-frame-indicator*` | ✅ sprint 3: `flash-current-frame!` (Print C-c) border pulse + geometry echo. |

## Missing — major projects (probably post-1.0 or explicit 1.0 goals)

| Feature | StumpWM | Assessment |
|---|---|---|
| **Multi-output (heads/screens)** | `snext`, `sprev`, `sother`, `refresh-heads`, per-head frame trees | ✅ sprint 4: per-head trees per group, `snext!`/`sprev!`/`sother!` (Print S / M-s), directional focus crosses bezels, hotplug adopts windows, `(set-head-mode! 'span)` for one spanning tree. |
| **Floating windows/groups** | `gnew-float`, `float-this`, `unfloat-this`, `flatten-floats` | ✅ sprint 6: `float-this!` toggle (Print F), `gnew-float!`, super+drag move/resize, floats above tiling, geometry survives group switches/hotplug (flatten-floats still ❌). |
| **Dynamic groups** (master/stack, i3-ish) | `gnew-dynamic`, `rotate-windows`, `exchange-with-master`, ... | StumpWM ships it, but it's a separate paradigm; layouts cover much of the need. Post-1.0. |
| **Key remapping per app** | `define-remapped-keys` (e.g. C-n→Down in browser) | Requires rewriting forwarded keys per focused app-id in input.rs. Medium Rust. |
| **Xwayland** | (implicit in X11) | ✅ sprint 5: embedded Xwayland; X11 apps are ordinary managed windows (title/class → title/app-id), DISPLAY exported, Wayland→X clipboard mirrored. Kill = polite close for X11 (shared client). |
| **Selection/clipboard commands** | `putsel`, `getsel` | ✅ sprint 3: `set-clipboard!` (putsel), prompt paste C-y/C-v + copy M-w, `copy-last-message!` (Print M-c); sprint 7: X11→Wayland clipboard direction synced too. |
| **Menus (select-from-menu)** | multi-line navigable menu with j/k/search | ✅ sprint 6: `(select-from-menu items on-select)` -- C-n/C-p/digits/typing-filters/Return/C-g; windowlist (Print l) and gselect (Print M-g) use it. |
| **Timers** | `run-with-timer`, `idle-hook` | ✅ sprint 3: `(wm-run-after ms thunk)` one-shot calloop timer; sprint 7: `(idle-ms)` primitive for building idle timers. |
| **Minor modes / modules ecosystem** | `load-module`, minor-modes | Guile modules already load from user config dir; formal minor-mode machinery unnecessary for 1.0. |

## Suggested 1.0.0 scope

1. **Sprint "window numbers + navigation"**: numbers 0–9 with `*`/`-`
   markers, select/pull by number, prev/reverse everywhere,
   other-window/gother/fother, move-focus + move-window/exchange
   (directional), only/fclear/splits-equally.
2. **Sprint "polish + help"**: which-key echo, describe-key/where-is/
   commands, colon eval prompt, quit-confirm, lastmsg, grename/gselect/
   gkill/gmove-and-follow, hooks system, marks.
3. **Sprint "Rust odds and ends"**: fullscreen, kill, banish/ratwarp,
   urgency, timer subr, frame indicator flash.
4. **Decide explicitly** (in or out for 1.0): multi-output, floating,
   Xwayland, remapped keys, menus. Everything in StumpWM not listed
   above is either covered or consciously excluded (mode-line, tray,
   selection, dynamic groups, minor modes).
