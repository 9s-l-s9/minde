# Configuration

Minde separates portable repository defaults from personal session policy.
The repository starts no wallpaper, bar, launcher, lock program, or personal
application. Guix Home may layer those choices on top.

## Load order

The Scheme entry point is selected in this order:

1. `MINDE_INIT`;
2. `$XDG_CONFIG_HOME/minde/init.scm` (normally `~/.config/minde/init.scm`);
3. the packaged/repository `scheme/init.scm` fallback.

`MINDE_SCHEME_DIR` selects the bundled module directory. The declarative
configuration comes from `MINDE_CONFIG`, otherwise from
`$MINDE_SCHEME_DIR/default-config.scm`. When `MINDE_SCHEME_DIR` is unset, it
comes from the directory of the `init.scm` that defines
`configuration-file-path` (normally the bundled file). Loading that file
from a personal entry point does not select a configuration beside the
personal file; set `MINDE_CONFIG` explicitly for a personal configuration.

If a personal init raises an exception, Minde logs its path, the Scheme
exception and source information, then restarts once with the bundled init
before acquiring the display or seat. The fresh process discards partially
installed hooks, timers and compositor settings, and restores the original
working directory and environment. The recovered session displays the error
for 30 seconds and keeps it in the session log. `MINDE_CONFIG` still selects
the declarative configuration. If the bundled init also fails, startup exits
with an error; selecting the bundle through a symlink does not retry it.

External effects such as files written or programs started by a failed init
cannot be undone. Put autostart programs in `handle-startup!`, which runs
after policy has loaded successfully.

**Ctrl+Alt+Backspace** immediately exits Minde through Rust, even if Scheme
keybindings fail, shortcuts are inhibited, or the session is locked. This is
an emergency exit and does not prompt applications to save their work.
Unsaved work may be lost. The declarative `emergency-exit` setting below can
rebind or disable it; Super+Q has no repository-default action.

## Declarative configuration

The configuration is one data expression, not evaluated code:

```scheme
(minde-config
 (version 1)
 (prefix () "Print")
 (emergency-exit (ctrl alt) "BackSpace")
 (bindings
  ("n" focus-next-window!)
  ("z" reload-configuration!)))
```

Binding targets must be registered commands with no arguments. Unknown fields,
versions, modifiers, commands, key names, and duplicate keys are errors.
Keys use case-sensitive XKB names such as `Return`, `space`, `Escape`, and
`XF86AudioPlay`. Binding modifiers are `C-` (Control), `M-` (Alt), `S-` (Shift),
and `s-` (Super). Their order is normalized: `M-C-g` becomes `C-M-g`.
XKB aliases are normalized too: `Page_Up` becomes `Prior`. Bindings that become
identical after normalization are duplicates. Prefix keys use the same XKB
names, with modifiers supplied separately in the prefix's list.

`emergency-exit` accepts a modifier list and an XKB key name in the same form
as `prefix`. For example, `(emergency-exit (ctrl alt shift) "Escape")` changes
the native exit to Ctrl+Alt+Shift+Escape. `(emergency-exit #f)` disables it.
Omitting the field selects Ctrl+Alt+BackSpace, including on reload. The
modifier combination must match exactly; Caps Lock and Num Lock do not count
as chord modifiers. XKB's `terminate:ctrl_alt_bksp` option also works with the
default, but does not bypass a disabled or rebound setting. Disabling the
chord removes the policy-independent keyboard escape hatch.

```sh
scripts/mindectl check-config path/to/config.scm
scripts/mindectl eval '(reload-configuration!)'
```

Offline checking knows the bundled commands, including the control commands
`desktop-snapshot` and `windows`. It does not load personal init files. To
validate against commands registered in the running session, use:

```sh
scripts/mindectl check-config path/to/config.scm --live
```

Both modes read the file as data, validate key names with libxkbcommon, and
require commands with no arguments.
Live checking sends the data to the session for validation; it does not apply
the configuration or invoke the bound commands. It uses the normal IPC
deadline, adjustable with `--timeout-ms`.

Reload builds and validates a candidate table before publishing it. Failure
leaves the active prefix, bindings and native emergency exit unchanged.

## Imperative personal layer

Personal Scheme can add launchers, hooks, layouts, placement rules and
`handle-startup!`. After installing personal bindings, call
`(register-configuration-layer!)` once so later declarative reloads retain that
baseline.

`bind-prefix-key!` and `bind-key!` replace the binding's help text along with
its action. Omitting the optional description makes the new binding
undocumented. Replacing a nested keymap with an ordinary command also removes
the old submap from help traversal. Removing a declarative override restores
the personal layer's binding, description, and submap relationship.

In command mode, the prefix map remains armed until Return, Escape, or `C-g`.
The literal-prefix escape (for example `C-t t` when `t` has no explicit
top-level binding) also leaves command mode completely. Outside command mode,
a prefix keysym bound inside a nested map behaves as an ordinary nested
binding and returns to normal input after the command finishes.

A modal keymap can declare a cleanup procedure with
`(set-keymap-cancel! keymap cleanup)`. It runs when the map is left, including
after an unbound key or a failed binding, and when the group, head, output
geometry or session-lock state interrupts selection. Child maps that share
the same cleanup procedure continue the operation without cancelling it.
Cleanup errors are logged and leave the keyboard usable.

`fselect!` and `expose-mode!` return such maps. Expose restores the original
layout before leaving its group or head; entering it repeatedly cannot
replace that snapshot with the temporary grid. Selection labels use `0`–`9`
and then `a`–`z`. Above 36 choices, labels have equal width (`00`, `01`, …,
`10`, …); type every character to select one. Escape, Return, `C-g`, or an
unbound key cancels. Older overlay timers cannot erase a newer selection.

The startup callback runs once after the first output becomes ready:

```scheme
(define (handle-startup!)
  (wm-spawn "swaybg -i /path/to/background")
  (wm-spawn "eww open bar"))
```

Editing a Guix Home service does not update the running session. Apply Home,
inspect `~/.config/minde/init.scm`, then log out and back in before judging
startup behavior.

## Window gaps

Minde provides StumpWM-style gaps directly through `(minde frames)`. Put this
in your personal `init.scm`, after loading Minde's bundled initialization:

```scheme
(configure-gaps! #:inner 5 #:outer 10 #:head 20)
(gaps-on!)
;; Optional personal binding; no shortcut is assigned by default.
(bind-prefix-key! "C-M-g" toggle-gaps! "toggle gaps")
```

The behavior follows [StumpWM's swm-gaps module](https://github.com/stumpwm/stumpwm-contrib/blob/master/util/swm-gaps/swm-gaps.lisp):

- `inner` is padding on **each** window edge. Two adjacent frames with inner
  padding 5 have 10 logical pixels between their display rectangles.
- `outer` adds padding at the outside edges of the tiled layout.
- `head` insets the whole tiled layout on each monitor. In span mode it insets
  the single synthetic head instead.

With the example above, the display rectangle starts 35 logical pixels from
the usable output edge: 20 head + 5 inner + 10 outer. The usual focus-border
reservation sits inside that rectangle, so client content starts another
3 pixels inward. Background surfaces remain visible in the gaps.

Gaps remain with a single window and with empty splits; there is no automatic
smart-gap rule. Fullscreen ignores gaps and restores the configured layout on
exit. Transient windows skip inner and outer padding. Floating windows retain
their geometry when gap settings change. Head padding applies to the usable
area after a bar's exclusive reservation, and does not resize the physical
output or move the bar. Sizes use logical pixels and follow output scaling.

`(gaps-off!)` disables gaps without forgetting sizes; `(gaps-on!)` enables
them; `(toggle-gaps!)` switches between the two. Repeated on/off calls are
idempotent. `(gap-settings)` returns an alist with `mode`, `enabled?`, `inner`,
`outer`, and `head`. The initial settings are 5, 10, 0, disabled.

`configure-gaps!` retains omitted sizes and the current enabled state. Values
must be exact nonnegative integers; an invalid update changes nothing. If a
frame is too small, effective padding is reduced to leave positive client
geometry. The requested sizes are retained for when more room is available.
Settings apply across groups and monitors; no per-group override is needed.

Live changes need no session restart once the updated Minde is installed:

```sh
scripts/mindectl eval '(begin (configure-gaps! #:inner 5 #:outer 10 #:head 20) (gaps-on!))'
scripts/mindectl eval '(toggle-gaps!)'
```

The existing `(set-gaps! inner outer)` remains a compatibility API: its inner
value is shared between frames (rounded down to an even total), and outer is
the total edge padding. It enables legacy mode with no head inset. Calling
`configure-gaps!` selects StumpWM semantics again, using the last StumpWM sizes
for omitted keywords. Gap settings currently belong in the personal Scheme
layer, not the declarative `minde-config` expression.

## Gamma and night light

The DRM backend advertises `wlr-gamma-control-unstable-v1` for clients such as
gammastep and wlsunset. Each output has at most one owning control. A rejected
duplicate, or a control invalidated by output removal, cannot change or
destroy another control's state. The nested backend does not advertise gamma
control.

A control can be created while its output is powered off. Minde retains the
latest table while DPMS, VT switching or output reinitialization prevents
hardware updates. After a modeset, power-on or session activation, it reapplies
the table after the first completed frame; blocking modesets on other outputs
during bandwidth fallback are followed by an immediate replay. If replay fails, the client receives
`failed` and the log names the output. Destroying a control or disconnecting its
client restores the original table, deferring restoration while the output is
inaccessible. A replacement control inherits that original baseline if the
restore is still pending.

Disabling or disconnecting an output invalidates its control; clients must
create a new one when the output returns. Tables must arrive through a regular
file or shared-memory descriptor with exactly three native-endian 16-bit
ramps of the advertised length. Pipes and device descriptors are rejected
before reading them on the compositor thread.

## Input device configuration

On the DRM/libinput backend, per-device pointer and touchpad behavior is
configured from Scheme. Two Rust primitives back this surface (they are
compositor primitives, not part of the versioned `(minde …)` module API):

`(wm-input-devices)` returns the libinput devices on the seat as
`((name capability …) …)`, where each capability is one of `"keyboard"`,
`"pointer"`, `"touch"`, `"tablet-tool"`, `"tablet-pad"`, `"gesture"`, or
`"switch"`. Under the nested winit backend there is no libinput context, so it
returns `()`.

`(wm-configure-input! match #:key tap-to-click natural-scroll accel-profile
click-method)` stores a configuration rule and applies it to matching devices.
`match` is `#t` (every device) or a substring of the device name. The keyword
settings, each left unchanged when omitted:

| Keyword | Values |
|---|---|
| `#:tap-to-click` | `#t` / `#f` |
| `#:natural-scroll` | `#t` / `#f` |
| `#:accel-profile` | `'flat` / `'adaptive` |
| `#:click-method` | `'button-areas` / `'clickfinger` |

A rule applies to every device already present and, since rules are stored, to
matching devices as they hotplug. A later rule with the same `match` replaces
the earlier one. Settings a device does not support are logged and skipped,
never fatal. Under winit the rule is stored but configures nothing.

```scheme
(define (handle-startup!)
  ;; Natural scrolling everywhere; tap-to-click and click-to-tap only on the
  ;; laptop touchpad.
  (wm-configure-input! #t #:natural-scroll #t)
  (wm-configure-input! "Touchpad"
                       #:tap-to-click #t
                       #:accel-profile 'adaptive
                       #:click-method 'clickfinger))
```

The optional hook `(handle-input-device-added! name capabilities)` fires after
each device arrives and its stored rules are applied, for imperative per-device
policy:

```scheme
(define (handle-input-device-added! name capabilities)
  (when (member "pointer" capabilities)
    (wm-configure-input! name #:accel-profile 'flat)))
```

### Keyboard layouts

minde builds one keymap from the standard `XKB_DEFAULT_*` variables. A comma
list gives several *layout groups* that share the keymap:

```sh
XKB_DEFAULT_LAYOUT=de,us XKB_DEFAULT_VARIANT=bone, minde
```

If xkbcommon cannot compile the requested keymap, minde logs all five
`XKB_DEFAULT_*` values and tries `evdev` rules, the `pc105` model and the `us`
layout with no variant or options. This fallback ignores the requested names
without changing the process environment. If the fallback also fails, startup
exits with an error before opening the desktop.

xkbcommon's own toggle options work unchanged
(`XKB_DEFAULT_OPTIONS=grp:win_space_toggle`, `grp:caps_toggle`, ...); the
compositor also exposes the groups to Scheme so a switch can sit under the
prefix key. `(keyboard-layouts)` lists the groups in keymap order as
`((name . "German (Bone)") (active . #t))` alists;
`(set-keyboard-layout! spec)` activates one by zero-based index, by the
symbol `next`/`prev` (cycling) or by a case-insensitive name prefix
(`"English"`). The focused client sees the change through the normal
modifiers event. Whenever the active group changes -- through
`set-keyboard-layout!` or an XKB toggle -- `(handle-keyboard-layout-changed!
name)` runs; the default shows the name in the message overlay, redefine it
for a bar or silence. A typical use is a keyboard whose firmware already
implements the layout (a Corne with Bone on a US host) next to the built-in
keyboard:

```scheme
(bind-prefix-key! "k" (lambda () (set-keyboard-layout! 'next)) "keyboard layout")
```

Both wrappers live in `(minde groups)` over the primitives
`wm-keyboard-layouts` and `wm-set-keyboard-layout!`. Per-device layouts
(one keymap per physical keyboard) are not implemented.

## Outputs and multiple monitors

Minde does not have an `outputs` section. Output layout is protocol-driven: the
compositor implements `wlr-output-management-unstable-v1` and any client of
that protocol may arrange the heads. Layout is owned by one external daemon,
started from `handle-startup!`; the compositor state is the single source of
truth, and disabled heads stay advertised so a daemon can re-enable them.

The recommended stack, all available from the development shell
(`manifest.scm`) or the in-repo channel (`guix build -L guix-channel shikane`):

| Tool | Role |
|---|---|
| `shikane` | profile daemon (packaged in `guix-channel/`); matches heads by model/serial, applies the first complete profile, reacts to hotplug |
| `kanshi` | alternative profile daemon, in Guix proper; same compositor behavior |
| `wdisplays` | GUI arranger, for authoring a layout interactively |
| `wlr-randr` | one-shot changes and inspection (`wlr-randr` lists heads and modes) |
| `wlopm` | `wlr-output-power-management` client (`wlopm --off '*'` turns screens off) |

### shikane

Spawn the daemon at startup:

```scheme
(define (handle-startup!)
  (wm-spawn "shikane"))
```

Its configuration lives in `~/.config/shikane/config.toml`. A profile is
selected when every connected head matches exactly one of its outputs, so
match on stable identity (model `m=`, serial `s=`) rather than connector name:

```toml
[[profile]]
name = "desk"

[[profile.output]]
search = "s=ABC123456"          # laptop panel, by serial
enable = true
mode = "1920x1080@60Hz"
position = "0,0"
scale = 1.0

[[profile.output]]
search = ["m=U2723QE", "v=DEL"] # external, model + vendor
enable = true
mode = "3840x2160@60Hz"
position = "1920,0"
scale = 1.5

[[profile]]
name = "laptop-only"

[[profile.output]]
search = "s=ABC123456"
enable = true
mode = "preferred"
position = "0,0"
```

To author a profile, arrange the heads with `wdisplays` (or `wlr-randr`),
then let shikane write it:

```sh
shikanectl export desk >> ~/.config/shikane/config.toml
shikanectl reload
```

`shikanectl switch NAME` applies a named profile by hand.

### kanshi (fallback)

kanshi is in Guix and needs no channel. Same idea, scfg syntax, matched by the
output description (`wlr-randr` prints it):

```
profile desk {
    output "Some Vendor Panel 0x1234" enable mode 1920x1080@60Hz position 0,0 scale 1
    output "DEL U2723QE ABC123" enable mode 3840x2160@60Hz position 1920,0 scale 1.5
}
```

Start it with `(wm-spawn "kanshi")` instead of shikane; run only one daemon.

### The Scheme side

The compositor keeps its head model in Scheme (`(wm-outputs)`,
`handle-heads-change!` in `(minde groups)`), so an accepted external change
reconciles into groups and frames exactly as a hotplug or resize does. The
relevant knobs, all exported by `(minde groups)`:

- `(set-head-mode! 'per-head)` (default) gives every group one frame tree per
  head, StumpWM style; `(set-head-mode! 'span)` uses a single tree over the
  union of all heads. See [Concepts](concepts.md).
- `(output-configuration-allowed?)` is the policy predicate consulted before
  an external client's test or apply request is honored. The default returns
  `#t`; redefine it in your init file to return `#f` to refuse external
  changes, or gate them on your own state.
- `(handle-output-configured!)` fires after a change was applied -- by an
  external client or by `configure-output!` -- for re-tiling, persisting or
  logging. The default does nothing.
- `(handle-output-configure-failed! name reason)` fires when a
  `configure-output!` request was rejected. The default logs it.

```scheme
(define (output-configuration-allowed?) #t)
(define (handle-output-configured!)
  (wm-log "output layout changed"))
```

The three hooks are compositor entry points looked up by plain top-level
name, like `handle-startup!`: a definition in your init file shadows the
default from `(minde groups)`.

#### Configuring outputs from Scheme

Without any daemon, the same apply path is reachable from Scheme --
imperatively from a hook, or interactively over IPC:

```sh
mindectl eval '(output-heads)'
mindectl eval '(configure-output! "DP-1" #:scale 1.5 #:position (quote (1920 0)))'
```

`(output-heads)` returns every known head, disabled ones included, as a list
of alists -- `name`, `enabled`, `make`, `model`, `serial`, `description`,
`mode` and `preferred-mode` as `(width height refresh-mhz)` (or `#f`),
`position` as `(x y)`, `scale`, `transform` (`normal`, `90`, `flipped-90`,
...), `adaptive-sync` (`#t`, `#f` or `unsupported`) and `modes`, the full
mode list. It is the raw per-connector view a wlr-output-management client
sees; `(wm-outputs)` remains the enabled heads' usable rectangles.

`(configure-output! name #:mode #:position #:scale #:transform #:enabled
#:adaptive-sync)` queues a change for the head called `name`; every omitted
keyword is left unchanged. `#:mode` accepts `'(w h refresh-mhz)`, `'(w h)`
(any refresh at that size) or a string `"1920x1080@60"`. An omitted or zero
refresh selects the preferred mode at that size, otherwise the highest
advertised refresh. `#:transform` accepts a symbol or rotation in degrees.
To change settings on a disabled head, include `#:enabled #t` in the same
request. Settings combined with `#:enabled #f` are rejected through
`handle-output-configure-failed!`.

The request is validated and applied
through the same revert-on-failure routine as external clients (unknown
head or mode, unsupported adaptive sync, disabling the last head and -- under
the nested winit backend -- mode changes are refused, in which case
`handle-output-configure-failed!` receives the reason). Because this is your
own policy speaking, `configure-output!` does **not** consult
`output-configuration-allowed?`; that predicate gates external clients only.
A malformed setting (`#:scale "big"`) is a Scheme error rather than a silent
no-op. The primitives underneath, `wm-output-heads` and
`wm-configure-output!` (name, alist), are listed by `mindectl help` like the
other `wm-*` gsubrs.

A daemon and `configure-output!` may coexist, but the last writer wins: a
profile daemon re-applies its profile on every head change, so use
`configure-output!` either without a daemon or for state the daemon does not
manage.

### Hardware status

As of this version, position, scale and transform apply on hardware and under
the nested winit backend. Real mode switching, disabling and re-enabling
heads, EDID-based make/model/serial, DPMS via
`wlr-output-power-management-v1` and adaptive sync are being added in
stages; until they land, a profile that changes the mode or disables a head is
refused (the daemon reports the apply as failed and tries the next variant),
and heads report `Unknown` identity, so match kanshi/shikane profiles by
connector name (`n=eDP-1`) on such builds. See
[hardware validation](hardware-validation.md) and the
[capability matrix](capability-matrix.md) for what a given build supports.

## Common environment variables

| Variable | Purpose |
|---|---|
| `MINDE_INIT` | Scheme entry point |
| `MINDE_SCHEME_DIR` | bundled module search directory |
| `MINDE_CONFIG` | declarative configuration file |
| `MINDE_TERMINAL` | terminal command; default `foot \|\| xterm` |
| `MINDE_LOG_FORMAT` | `human` or newline-delimited `json` |
| `MINDE_UNSAFE_REPL` | opt into the unsafe threaded development REPL |
| `XKB_DEFAULT_*` | keyboard rules, model, layout, variant and options |

`MINDE_FULL_KEYMAP=1` retains the historical feature-oriented map. It is an
escape hatch for established configurations, not the portable release
default.

## Persistent data

Layouts and placement rules use versioned Scheme data and atomic replacement.
The defaults are under `~/.config/minde/`; tests override them with
`MINDE_LAYOUTS_FILE` and `MINDE_RULES_FILE`. Desktop dumps are explicit
user-requested files. Before the first release, incompatible old state is
intentionally rejected rather than migrated silently.

`save-layout!` writes the named snapshot and retains earlier saved snapshots;
it does not copy other configuration presets into the file. Definitions made
with `define-layout!` take precedence over same-named saved layouts on load,
including built-ins stored by older versions. Equivalent placement rules are
deduplicated while preserving first-match order and distinct policies.

An unreadable or invalid layouts/rules file is preserved. Failed loads leave
the registry unchanged, log the filename and error, and display a diagnostic
when the first output becomes available. Saves return `#f` until you repair
the file and call `load-layouts!` or `load-placement-rules!` successfully, or
move the file aside. Every save also checks for invalid external edits made
since loading. Failed `save-layout!`, `remember!` and `forget!` operations leave
the in-memory registry unchanged and never report success.
