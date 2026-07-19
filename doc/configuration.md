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
