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

Key bindings are StumpWM-style behind a prefix (default: `Print`, like
the author's StumpWM; change with `(set-prefix-key! '(ctrl) "t")`).
Pressing the prefix key again forwards it literally to the client.
Prefix map (see `scheme/init.scm`):

| key | action | | key | action |
|-----|--------|-|-----|--------|
| `Return` | terminal (alacritty) | | `r` | run prompt (native, TAB-completes PATH) |
| `b` | browser (zen/chromium) | | `e` / `E` | lem / emacsclient |
| `v` / `h` | vsplit / hsplit | | `c` | remove split |
| `n` | next frame | | `f` | next window (group-wide) |
| `p` | pull next hidden window | | `o` | next window in this frame |
| `k` / `d` | close window | | `l` | windowlist (menu) |
| `g` | next group | | `G` | new group |
| `m` | move window to next group | | `y` | window info echo |
| `a` | ask AI (prompt + echo) | | `T` | add TODO (prompt) |
| `w` | voice dictate | | `i` | eww widgets |
| `A` | agents submap (c/d/o/p) | | `P` | misc submap (s/w/a) |
| `s` | resize mode (iresize) | | `F` | float/unfloat window |
| `M-F` | apply layout (prompt) | | `M-g` | switch group (menu) |
| `0`–`9` | select window by number | | `C-0`–`C-9` | pull window by number |
| arrows | move focus directionally | | `M-`arrows | move window directionally |
| `S-`arrows | swap with neighbor frame | | `Tab` / `S-Tab` | last window / last frame |
| `u` | last group (gother) | | `W` | numbered window list echo |
| `C` | only (collapse splits) | | `Delete` | fclear (empty this frame) |
| `C-f` `C-p` `C-n` `C-o` | reverse of f/p/n/o | | `?` | which-key: list bindings |
| `F1` | describe next key | | `colon` | eval Scheme (prompt) |
| `C-m` | last message again | | `x` / `M-x` / `C-x` | mark / pull marked / clear |
| `M-m` | move window + follow | | | |
| `M-f` | fullscreen toggle | | `K` | kill window (force, drops client) |
| `B` | banish pointer | | `C-c` | flash current frame |
| `C-u` | jump to urgent window | | `M-c` | copy last message |
| `S` | next head (monitor) | | `M-s` | last head (monitor) |
| `z` | command mode (`RET`/`C-g` exits) | | | |
| `C-l` | frame windowlist (menu) | | `M-l` | windowlist by class (menu) |
| `M-p` | pull from windowlist (menu) | | `M-w` | select window by name (prompt) |
| `M-t` | select floating window (menu) | | `C-r` | redisplay windows |
| `M-G` | groups submap (see below) | | `M-G d`/`D` | new dynamic group (fg/bg) |
| `M-G r`/`x` | rotate / exchange with master | | `M-G l`/`S`/`t` | layout / ratio / retile |
| `j` | fselect: jump to numbered frame | | `M-e` | expose: pick window from grid |
| `M-o` | sibling frame | | `M-h` / `M-v` | h/vsplit uniformly (prompt) |
| `C-q` | send next key to window | | `F2` | describe command (prompt) |
| `F3` | list all commands | | `F4` | describe function (prompt) |
| `F5` | where-is (doc search) | | `F6` | describe variable (prompt) |
| `R` | reload init.scm | | `L` / `Q` | lock / quit (asks) |

Xwayland: an embedded X server starts automatically; X11-only apps
(X-built emacs, xterm, legacy tools) appear as ordinary managed windows
— frames, numbers, placement rules and hooks treat them identically
(their X11 class is the app-id). `DISPLAY` is exported to children.
Copying on the Wayland side (including `set-clipboard!`) pastes into X11
apps. Caveats: `kill-window!` politely closes X11 windows (all X apps
share one client connection), and X11 primary selection/DnD are not
synced.

Floating windows: `Print F` toggles the focused window between tiled
and floating (StumpWM float-this/unfloat-this). Floats keep arbitrary
geometry, render above the tiling, and move/resize with
**super+left-drag** / **super+right-drag**. `(gnew-float! "name")`
creates a group where every window floats on map. Floats stay in the
windowlist/number/cycling rotation; pulling one (`C-0`–`C-9`,
pull-marked) re-tiles it into the current frame.

Group management (`Print M-G` submap, StumpWM parity): `n`/`N` create a
tiling/float group in the background (gnewbg/gnewbg-float — `G` and
`gnew!` now switch, like StumpWM), `m` merges another group's windows
here (gmerge), `o` kills every other group (gkill-other), `M` moves the
marked windows to a chosen group (gmove-marked), `f`/`b` take the
current window to the next/previous group and follow
(gnext/gprev-with-window), `k`/`K` politely close every window of this
group / of all other groups, `g` echoes the group list with window
counts (groups/vgroups).

Dynamic groups (StumpWM dynamic-group.lisp): `M-G d`/`D` create an
auto-tiling group (foreground/background). The newest window is the
master, taking 2/3 of the head on the left; the rest stack evenly
beside it, and every map/unmap/gmove/float retiles automatically —
manual splits are refused there. In the `M-G` submap: `r`/`C-r` rotate
all windows through the master position, `s` rotates just the stack,
`x` exchanges the focused window with the master, `l` picks the master
position (left/right/top/bottom, per group and head), `S` sets the
master ratio (0.1–0.9), `t` forces a retile. Colon-callable:
`(change-default-layout! 'right)`, `(change-default-split-ratio! 1/2)`,
`hnext!`/`hprev!` (= next/prev head) and
`fnext-in-head!`/`fprev-in-head!` (= per-head frame cycling — our
frames are per-head trees already).

`Print P y` toggles always-show: the window
follows every group switch (StumpWM toggle-always-show). `Print P i`
echoes the focused window's properties (id/title/class/number/float
geometry/flags); `Print P d` echoes the date, `Print P V` the version,
`Print P M` the modifier layout.

Frames & placement (StumpWM parity): `Print j` (fselect) draws each
frame's number in its corner — press the digit to jump. `Print M-e`
(expose) tiles every window of the head one-per-frame in a numbered
grid; a digit picks one and the previous layout is restored exactly
(windows back in their frames). `Print M-o` focuses the sibling frame.
`Print P u` unmaximizes the focused window (2/3 of its frame, CSD
corners back); `Print P g` sets its gravity (center/top-left/...).
`Print P R` **remembers** the focused window — a persistent placement
rule (app-id → this group + frame) written to
`~/.config/minde/rules.scm` and reloaded at startup; `Print P F`
forgets it. `add-placement-rule!` gains `#:lock?` (StumpWM `:lock`,
default `#t` = apply on map; `#f` rules only fire via
place-existing-windows) and `#:raise?` (alias of `#:follow?`).
`Print P D` / `Print P O` dump/restore the whole desktop (groups, frame
layouts, window assignments, float geometry) to a file. `Print M-h` /
`Print M-v` split the current frame into N equal parts.

Odds and ends: `Print P n` renames the focused window (StumpWM
`title`), `Print P p` re-applies placement rules to existing windows,
`Print P f` flattens this group's floats back into the frame, `Print P
t` toggles always-on-top. From Scheme: `(window-send-string "text")`
types into the focused window (chars must exist at shift level 0/1 of
the active layout), `(ratclick! 1)` clicks, `(idle-ms)` returns
milliseconds since the last input (build idle timers with
`wm-run-after`). The clipboard now syncs both directions with X11 apps.

Menus: `select-from-menu` renders a multi-line list in the message
area — `C-n`/`C-p`/arrows navigate, digits jump, typing filters,
`Return` selects, `C-g` aborts. The windowlist (`l`) and group switcher
(`M-g`) use it.

Multi-monitor: every group has a frame tree per head (StumpWM style);
`Print S` cycles heads, `Print M-s` toggles, and directional focus
(`Print` arrows) crosses monitor edges. Unplugging a monitor moves its
windows into the surviving head. Prefer one big tree spanning all
monitors? `(set-head-mode! 'span)` in init.scm (`'per-head` to go back);
`(wm-outputs)` lists heads as `(id x y w h name)`.

`?` works while the prefix or any submap is armed and keeps it armed.
`Print P r` renames the current group, `Print P k` deletes it (its
windows move to the next group).

## Hooks

`(minde hooks)` gives user configs StumpWM-style event hooks:
`(add-hook!* 'focus-window (lambda (id) ...))`. Fired hooks:
`new-window (id title app-id)`, `destroy-window (id)`, `focus-window
(id-or-#f)`, `focus-frame (x y w h)`, `focus-group (name)`, `message
(text)`. A throwing hook is logged and skipped, never fatal.

Keymap keys accept StumpWM-style modifier prefixes: `C-` (ctrl), `M-`
(alt), `S-` (shift), `s-` (super), e.g. `(bind-prefix-key! "M-Left" ...)`.
A modifier-prefixed binding wins over the bare keysym name.

`super+q` also quits. Groups " I ", " II ", " III " exist at startup.
The window model is StumpWM's (emacs-like): frames are panes, the group's
window list is the buffer list; `f` cycles it group-wide, `p` digs out
hidden windows, one window per frame is visible at a time.

## Gaps, resize, layouts, placement rules

- **Gaps**: `(set-gaps! inner outer)` (from `(minde frames)`) puts
  `inner` px of space between frames and `outer` px against the screen
  edge. Off by default; see the commented example in init.scm.
- **Resize**: `Print s` enters StumpWM-style iresize -- arrows/`hjkl`
  move the nearest split divider by 30px, `b`/`=` balances all frames,
  `RET`/`ESC` exits. The focus border turns blue while the mode is
  armed. One-shot from Scheme: `(resize-frame! 'down 30)`,
  `(balance-frames!)`.
- **Layouts** (`(minde layouts)`): named frame-tree presets. A spec
  is `'leaf` or `(hsplit ratio a b)` / `(vsplit ratio a b)`; presets
  `main-side`, `main-stack`, `grid4`, `full` ship in init.scm.
  `Print F` prompts (TAB completes) and applies one, redistributing the
  group's windows; `Print P s` saves the live tree under a name to
  `~/.config/minde/layouts.scm`, reloaded at startup. From Scheme:
  `(define-layout! name spec)`, `(apply-layout! name)`,
  `(dump-layout-spec)`.
- **Placement rules**: `(add-placement-rule! "zen" #:group "II"
  #:frame 0 #:follow? #f)` routes windows whose app-id or title contains
  the matcher string to a group/frame when they map (StumpWM
  `define-frame-preference`). First matching rule wins.

## Status line for bars (eww)

minde writes `"[I]  II  III | window title"` to
`$XDG_RUNTIME_DIR/minde-status` whenever it changes (also available
as `(status-line)` over the REPL socket / `minde-cmd`). `doc/eww/`
contains a minimal working eww bar consuming it with `deflisten` +
`tail -F`; because the bar sets a layer-shell exclusive zone, the frame
tree automatically tiles into the remaining space.

## Layer shell

minde implements `zwlr_layer_shell_v1`: launchers (fuzzel), wallpaper
(swaybg), lockers (swaylock), and bars/widgets (eww) work. A layer
surface with an exclusive zone (e.g. a docked eww bar) reserves screen
space -- the frame tree automatically tiles into the remaining area, so
an eww mode-line never overlaps windows. Exclusive-keyboard layers
(fuzzel, swaylock) take the keyboard while open; focus returns to the
current frame's window when they close.

## Native input prompt

`(read-one-line prompt on-submit #:key completions initial history)` from
`(minde input)` prompts in the message overlay -- StumpWM's
`read-one-line`, no external launcher. `Print r/a/T/l/G` use it. Editing
keys (StumpWM `*input-map*` subset): BackSpace, `C-d`/Delete, `C-f`/`C-b`
(Right/Left), `M-f`/`M-b` (words), `C-a`/`C-e` (Home/End), `C-k`, `C-u`,
`M-d`/`M-BackSpace` (kill words), `C-p`/`C-n` (Up/Down: history), TAB /
Shift-TAB (prefix-completion cycle), `C-y`/`C-v` (paste the clipboard),
`M-w` (copy the buffer), RET submit, `C-g`/ESC abort. Held keys
auto-repeat inside prompts, menus and armed keymaps: Wayland repeat is
client-side, so the compositor re-fires consumed keys itself
(`wm-set-key-repeat`, 600 ms delay / 40 ms interval). While the prefix
key is armed, the focus border turns red (`wm-border-color`, colors
configurable in init.scm).

## Remapped keys, key synthesis & help

`(define-remapped-keys! '(("zen" ("C-n" . "Down") ("C-p" . "Up"))))`
translates keys per application (StumpWM `define-remapped-keys`): when
the focused window's app-id matches the regex and the pressed key isn't
taken by a binding, the mapped key is synthesized into the window
instead. `(toggle-remapped-keys!)` / `(unbind-remapped-keys!)` manage
the table. Building blocks: `(send-key "C-M-x")` / `(meta ...)` /
`(send-escape)` synthesize any spec into the focused window
(`wm-send-key` resolves it in the active xkb layout, wrapping modifier
presses around it); `Print C-q` sends the next pressed key literally.
`(ratrelwarp dx dy)` warps the pointer relatively.

Help family: `Print F2` describe-command (completes over binding docs),
`F3` lists all documented bindings, `F4`/`F6` describe a live Guile
function/variable, `F5` where-is searches docs for a substring, and
`(which-key-mode!)` auto-echoes an armed keymap's bindings after ~1 s
(the same text `?` shows on demand). A keybinding error is kept in
`%last-unhandled-error`; `(copy-unhandled-error!)` puts it on the
clipboard. `(load-module! "ice-9 format")` pulls a Guile module into
the session; `(restart-soft)` = reload init.

## Message area

`(wm-message "text")` (optional second arg: timeout in ms, default 5000)
shows a centered gruvbox echo box rendered in-compositor -- StumpWM's
message window. Unbound prefix keys, group switches, and `Print y` use
it. From shell scripts, `minde-msg "text"` / `minde-cmd '(expr)'`
(installed in the package's `bin/`, or `scripts/` in the repo) talk to
the running compositor via the REPL socket -- that's how the `a`/`T`/`l`
prompt workflows report back.

Configuration: minde loads `$MINDE_INIT`, else
`~/.config/minde/init.scm`, else the repo's `scheme/init.scm`. The
bundled modules `(minde frames)` / `(minde groups)` are always on the
load path. Keyboard layout comes from the standard `XKB_DEFAULT_*`
environment variables, e.g. `XKB_DEFAULT_LAYOUT=de XKB_DEFAULT_VARIANT=bone`.

Layout policy lives in `scheme/minde/frames.scm` and
`scheme/minde/groups.scm`; tests: `guile -L scheme tests/frames-test.scm`
(also `groups-test.scm`, `keys-test.scm`).

Note when running nested inside StumpWM: StumpWM intercepts the prefix you
give it (its own default is `C-t`) -- forward with StumpWM's `C-t t`, or
pick a prefix StumpWM doesn't grab.

`wm-handle-key` is looked up by name on every keypress rather than cached,
so redefining it (or the keybinding table) from a live REPL takes effect
immediately.

## Running standalone (TTY / login session)

minde auto-picks its backend: nested (winit) when `DISPLAY` or
`WAYLAND_DISPLAY` is set, DRM/libinput (`--tty`) otherwise; force with
`--winit`/`--tty`.

First live test from a text console (Ctrl+Alt+F3, log in):

```sh
cd ~/Projects/minde
guix shell -m manifest.scm -- sh -c \
  'XKB_DEFAULT_LAYOUT=de XKB_DEFAULT_VARIANT=bone cargo run --release -- --tty'
```

Expected: cursor visible and moving; `C-t Return` opens a terminal; typing
follows your layout; splits/groups behave as nested; brightness/volume keys
work; `Ctrl+Alt+F7` VT-switches back to your X session (minde keeps
running); `super+q` exits back to the console. **Rollback**: your StumpWM
session is untouched — if minde wedges, switch VTs and `pkill minde`.

To select minde at login (SDDM): build the package and add it to the
system profile — see `doc/guix-system.scm`. One-time before building:

```sh
guix shell -m manifest.scm -- cargo vendor vendor   # offline crate mirror
guix build -f guix.scm                              # sanity check
```

The package installs `bin/minde-session` (env wrapper: de/bone keymap
defaults, EGL vendor path, bundled Scheme modules) and
`share/wayland-sessions/minde.desktop` for the SDDM menu.

Autostart programs: add commands to `%autostart` in your init.scm — they
run once when the first output is up (`wm-on-startup`). Media keys
(brightness/volume) and `C-t L` (swaylock) are bound by default.

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

## Development: ./check

One command verifies everything the sandbox can reach:

```sh
guix shell -m manifest.scm xorg-server xdotool imagemagick foot -- ./check
```

build + clippy (informational) + a borrow lint (`tests/lint-borrows.sh`,
guards the layer-map RefCell double-borrow class that once froze the TTY)
+ all Guile suites + a scripted Xvfb e2e (`tests/e2e.sh`: prompt, TAB
completion, spawn, reload, splits; screenshots land in /tmp/minde-e2e).

What it cannot reach: the udev/DRM backend at runtime. Anything touching
`src/udev.rs` needs a live `./debug-tty.sh` from a spare VT **before**
`guix system reconfigure`. If a session does crash, evidence survives in
`~/.local/state/minde/`: `session.log` (full compositor output, via
the session wrapper) and `crash.log` (panic + backtrace, via the
binary's panic hook).
