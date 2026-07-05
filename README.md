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

## License and support

Project-owned code is licensed under GPL-3.0-or-later. Code adapted from
Smithay examples remains available under the MIT license; exact provenance is
recorded in [NOTICE](NOTICE). The complete texts are in [LICENSES](LICENSES),
with [COPYING](COPYING) describing how they apply.

This is a pre-1.0 project maintained by one person. See [SUPPORT.md](SUPPORT.md)
for the support scope, [SECURITY.md](SECURITY.md) for private vulnerability
reporting, and [CONTRIBUTING.md](CONTRIBUTING.md) for local verification.

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

Reusable Guile code is split into two independently testable packages:
`guile-minde-foundation` provides geometry, binary trees, serialization,
hooks, and key notation; `guile-minde-ui` provides prompt and menu state
machines with injected display operations. Run `make check-foundation` and
`make check-ui` without a display, or build their Guix definitions from
`guix/foundation.scm` and `guix/ui.scm`.
The module inventory and adapter contract are documented in
[`doc/reusable-packages.md`](doc/reusable-packages.md).

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

Key bindings use a prefix-only map (portable default: `C-t`; change it in the
declarative configuration).
Pressing the prefix key again forwards it literally to the client.
Prefix map (see `scheme/init.scm`):

| key | action | | key | action |
|-----|--------|-|-----|--------|
| `h/j/k/l` | focus frame | | `H/J/K/L` | move window |
| `f H/J/K/L` | exchange windows | | `n/N` | next/previous window |
| `p/P` | pull next/previous hidden | | `0`–`9` | select numbered window |
| `w 0`–`w 9` | pull numbered window | | `Return` | configured terminal |
| `r` | run prompt | | `Space` | command palette |
| `colon` | evaluate Scheme | | `?` | contextual help |
| `w` | window submap | | `f` | frame submap |
| `g` | group submap | | `m` | layout/mode submap |
| `s` | session submap | | | |

Nested direct bindings include `w 0`–`w 9` to pull a numbered window,
`f 0`–`f 9` to select a numbered frame, and `f H/J/K/L` to exchange windows
between frames. Entering `w` displays the numbered window list; entering `f`
draws numbered frame overlays. `?` while any map is armed shows that map's
documented keys.

Set `MINDE_TERMINAL` to select a terminal; the fallback is `foot || xterm`.
Application launchers, keyboard layout, wallpaper, bars, locks, and autostart
are intentionally user policy and are not part of the repository default.
Set `MINDE_FULL_KEYMAP=1` before loading `scheme/init.scm` to retain the
complete feature-oriented keymap instead of replacing it with the reduced
portable map. This is intended for established personal configurations.

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
**super+left-drag** / **super+right-drag**. `(create-floating-group! "name")`
creates a group where every window floats on map. Floats stay in the
windowlist/number/cycling rotation; pulling one (`C-0`–`C-9`,
pull-marked) re-tiles it into the current frame.

Group management (`Print M-G` submap, StumpWM parity): `n`/`N` create a
tiling/float group in the background (gnewbg/gnewbg-float — `G` and
`create-group!` now switch, like StumpWM), `m` merges another group's windows
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
`focus-next-head!`/`focus-previous-head!` (= next/prev head) and
`focus-next-frame!`/`focus-previous-frame!` (= per-head frame cycling — our
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
`(add-event-hook! 'focus-window (lambda (id) ...))`. Fired hooks:
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

## Status for bars (eww)

minde writes `"[I]  II  III | window title"` to
`$XDG_RUNTIME_DIR/minde-status` whenever it changes (also available
as `(status-line)` over main-thread IPC). It also atomically publishes
schema-v1 JSON at `$XDG_RUNTIME_DIR/minde/status.json`. Prefer
`mindectl query state --json` or `mindectl subscribe --json` for new
bars; the schema includes groups, focused window, urgency, outputs, runtime,
and layout. See [`doc/diagnostics.md`](doc/diagnostics.md).

## Layer shell

minde implements `zwlr_layer_shell_v1`: launchers (fuzzel), wallpaper
(swaybg), and bars/widgets (eww) work. A layer
surface with an exclusive zone (e.g. a docked eww bar) reserves screen
space -- the frame tree automatically tiles into the remaining area, so
an eww mode-line never overlaps windows. Exclusive-keyboard layers
(fuzzel) take the keyboard while open; focus returns to the
current frame's window when they close.

Modern swaylock does not use layer shell; it requires
`ext-session-lock-v1`, which minde does not yet implement. The application
matrix reports that limitation explicitly instead of treating swaylock as a
passing layer-shell client.

## Native input prompt

`(read-one-line prompt on-submit #:key completions initial history)` from
`(minde ui prompt)` prompts in the message overlay -- StumpWM's
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
the running compositor via main-thread IPC -- that's how the `a`/`T`/`l`
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

The package installs `bin/minde-session` (EGL vendor path and bundled
Scheme modules) and
`share/wayland-sessions/minde.desktop` for the SDDM menu.

User configuration may replace `handle-startup!`, which runs once when the
first output is ready. The portable default starts nothing automatically.
Media keys remain available when their corresponding command-line tools are
installed.

## Runtime evaluation

The supported control path is a local Unix socket at
`$XDG_RUNTIME_DIR/minde-ipc.sock` (or `/tmp/minde-ipc.sock` when the
runtime directory is unset). Requests are evaluated by calloop on the
compositor thread, so configuration and window-policy mutation are serialized:

```sh
scripts/mindectl eval '(current-group-name)'
scripts/mindectl eval '(reload-configuration!)'
scripts/mindectl query state --json
timeout 5 scripts/mindectl subscribe --json
```

For exceptional interactive development, `MINDE_UNSAFE_REPL=1` restores
the old Guile REPL at `$XDG_RUNTIME_DIR/minde-repl.sock`. It is explicitly
unsafe because it evaluates on a separate thread; do not use it for normal
automation or state mutation.

## Development checks

One command verifies everything the sandbox can reach:

```sh
guix shell -m manifest.scm xorg-server xdotool imagemagick jq util-linux \
  foot xterm wl-clipboard shellcheck -- make check-all
```

This runs locked Rust build/tests, formatting, fatal Clippy, ShellCheck, the
borrow lint (`tests/lint-borrows.sh`), all Guile suites, documentation sanity,
and both scripted Xvfb keymap sessions plus the small foot/clipboard/xterm
application gate. Screenshots and logs land below `/tmp/minde-*`.
`./check` remains a wrapper for the same target.

For a faster implementation loop, enter the Guix shell once and run the
independent targets separately (or concurrently): `make check-rust`,
`make check-scheme`, `make check-static`, and `make check-e2e`. Packaging and
DRM validation are separate gates: `make check-package` and
`make check-hardware`. Heavy application checks are sequential, opt-in batches;
see [`doc/application-testing.md`](doc/application-testing.md) before running
them. The full release roadmap and per-sprint owner checks are in
`doc/release-roadmap.md`.

What it cannot reach: the udev/DRM backend at runtime. Anything touching
`src/udev.rs` needs a live `./debug-tty.sh` from a spare VT **before**
`guix system reconfigure`. If a session does crash, evidence survives in
`~/.local/state/minde/`: `session.log` and `session.previous.log` (the two
retained compositor sessions) plus `crash.log` (panic + backtrace). Scheme
keybinding, reload, and IPC failures also include Guile backtraces. Set
`MINDE_LOG_FORMAT=json` before startup for newline-delimited structured
tracing, or create a redacted bundle with
`mindectl report --output /tmp/minde-report`.
