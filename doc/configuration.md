# Configuration

Minde separates portable repository defaults from personal session policy.
The repository starts no wallpaper, bar, launcher, lock program, or personal
application. Guix Home may layer those choices on top.

## Load order

The Scheme entry point is selected in this order:

1. `MINDE_INIT`;
2. `~/.config/minde/init.scm`;
3. the packaged/repository `scheme/init.scm` fallback.

`MINDE_SCHEME_DIR` selects the bundled module directory. The declarative
configuration comes from `MINDE_CONFIG`, otherwise
`scheme/default-config.scm` beside the selected entry point.

## Declarative configuration

The configuration is one data expression, not evaluated code:

```scheme
(minde-config
 (version 1)
 (prefix () "Print")
 (bindings
  ("n" focus-next-window!)
  ("z" reload-configuration!)))
```

Binding targets must be registered commands with no arguments. Unknown fields,
versions, modifiers, commands, and duplicate keys are errors.

```sh
scripts/mindectl check-config path/to/config.scm
scripts/mindectl eval '(reload-configuration!)'
```

Reload builds and validates a candidate table before publishing it. Failure
leaves the active prefix and bindings unchanged.

## Imperative personal layer

Personal Scheme can add launchers, hooks, layouts, placement rules and
`handle-startup!`. After installing personal bindings, call
`(register-configuration-layer!)` once so later declarative reloads retain that
baseline.

The startup callback runs once after the first output becomes ready:

```scheme
(define (handle-startup!)
  (wm-spawn "swaybg -i /path/to/background")
  (wm-spawn "eww open bar"))
```

Editing a Guix Home service does not update the running session. Apply Home,
inspect `~/.config/minde/init.scm`, then log out and back in before judging
startup behavior.

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
reconciles into groups and frames exactly as a hotplug or resize does. Three
knobs are relevant:

- `(set-head-mode! 'per-head)` (default) gives every group one frame tree per
  head, StumpWM style; `(set-head-mode! 'span)` uses a single tree over the
  union of all heads. See [Concepts](concepts.md).
- `(output-configuration-allowed?)` is an optional predicate consulted before
  a client's apply request is honored. Unbound (the default) means accept;
  define it to return `#f` to refuse external changes, or gate them on your
  own state.
- `(handle-output-configured!)` is an optional hook that fires after an
  external change was applied, for re-tiling, persisting or logging.

```scheme
(define (output-configuration-allowed?) #t)
(define (handle-output-configured!)
  (wm-log "output layout changed by an external client"))
```

Both are compositor entry points looked up by plain top-level name in the
init file, like `handle-startup!`; neither is part of the versioned
`(minde …)` module API.

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
